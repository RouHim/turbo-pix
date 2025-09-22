mod cache;
mod config;
mod db;
mod indexer;
mod scheduler;
mod web;
mod web_handlers;

use actix_web::{
    middleware::Logger,
    web::{get, scope, Data},
    App, HttpServer,
};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tracing::{info, warn};

use cache::{CacheManager, MemoryCache};
use scheduler::PhotoScheduler;
use web_handlers::ThumbnailService;
use web::static_handler::serve_static_asset;

#[actix_web::main]
async fn main() -> io::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Load configuration
    let config = config::Config::from_env().expect("Failed to load configuration");

    info!(
        "Starting TurboPix server on {}:{}",
        config.host, config.port
    );
    info!("Photo paths: {:?}", config.photo_paths);
    info!("Database: {}", config.db_path);

    // Create database pool
    let db_pool = db::create_db_pool(&config.db_path).expect("Failed to create database pool");

    info!("Database initialized successfully");

    // Initialize thumbnail system
    let memory_cache = MemoryCache::new(
        config.cache.memory_cache_size,
        config.cache.memory_cache_max_size_mb,
    );
    let thumbnail_service = Arc::new(ThumbnailService::new(
        &config,
        memory_cache.clone(),
        db_pool.clone(),
    ));
    info!("Thumbnail system initialized");

    // Initialize cache manager
    let cache_manager = CacheManager::new(
        memory_cache,
        config.cache.thumbnail_cache_path.clone().into(),
    );

    // Start photo scheduler with cache manager
    let photo_paths: Vec<PathBuf> = config.photo_paths.iter().map(PathBuf::from).collect();
    let photo_scheduler = PhotoScheduler::new(photo_paths, db_pool.clone(), cache_manager);
    let _scheduler_handle = photo_scheduler.start();
    info!("Photo scheduler started");

    // Run startup rescan instead of manual scan
    info!("Running startup photo rescan and cleanup...");
    if let Err(e) = photo_scheduler.run_startup_rescan().await {
        tracing::error!("Startup rescan failed: {}", e);
    }

    let host = config.host.clone();
    let port = config.port;
    let workers = config.workers;

    info!(
        "Server started successfully, listening on {}:{}",
        host, port
    );

    // Start HTTP server
    let server = HttpServer::new(move || {
        App::new()
            .app_data(Data::new(db_pool.clone()))
            .app_data(Data::new(config.clone()))
            .app_data(Data::new(thumbnail_service.clone()))
            .wrap(web::middleware::cors_headers())
            .wrap(Logger::default())
            .configure(web::routes::configure_routes)
            // Embedded static file serving - these should come after API routes
            .service(
                scope("")
                    .route("/", get().to(serve_static_asset))
                    .route("/index.html", get().to(serve_static_asset))
                    .route("/css/{filename}", get().to(serve_static_asset))
                    .route("/js/{filename}", get().to(serve_static_asset)),
            )
    })
    .workers(workers)
    .bind((host, port))?
    .run();

    // Graceful shutdown handling
    let server_handle = server.handle();
    let photo_scheduler_clone = photo_scheduler.clone();
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                warn!("Received shutdown signal, stopping server gracefully...");

                // Perform cleanup tasks
                info!("Running shutdown cleanup...");
                if let Err(e) = photo_scheduler_clone.shutdown_cleanup().await {
                    warn!("Shutdown cleanup failed: {}", e);
                }

                server_handle.stop(true).await;
            }
            Err(err) => {
                warn!("Unable to listen for shutdown signal: {}", err);
            }
        }
    });

    server.await
}
