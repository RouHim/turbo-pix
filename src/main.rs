use log::{error, info};
use std::error::Error;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use warp::Filter;

use turbo_pix::cache_manager::CacheManager;
use turbo_pix::config;
use turbo_pix::db;
use turbo_pix::db_pool;
use turbo_pix::handlers_collage::build_collage_routes;
use turbo_pix::handlers_config::build_config_routes;
use turbo_pix::handlers_health::build_health_routes;
use turbo_pix::handlers_housekeeping::build_housekeeping_routes;
use turbo_pix::handlers_indexing::build_indexing_routes;
use turbo_pix::handlers_photo::build_photo_routes;
use turbo_pix::handlers_saved_searches::build_saved_searches_routes;
use turbo_pix::handlers_search::build_search_routes;
use turbo_pix::handlers_static::build_static_routes;
use turbo_pix::handlers_thumbnail::build_thumbnail_routes;
use turbo_pix::scheduler::PhotoScheduler;
use turbo_pix::semantic_search::{self, SemanticSearch, SemanticSearchEngine};
use turbo_pix::thumbnail_generator::ThumbnailGenerator;
use turbo_pix::video_processor;
use turbo_pix::warp_helpers::{handle_rejection, require_same_origin};

// Avoid musl's default allocator due to lackluster performance
// https://nickb.dev/blog/default-musl-allocator-considered-harmful-to-performance
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let config = config::Config::from_env()?;

    // Handle --download-models flag
    if std::env::args().any(|arg| arg == "--download-models") {
        info!("Downloading AI models...");
        semantic_search::download_models(&config.data_path)?;
        info!("Download complete. You can now run tests.");
        return Ok(());
    }

    // Fail fast if ffmpeg or ffprobe binaries are not available
    if let Err(e) = video_processor::verify_ffmpeg_available() {
        error!("{}", e);
        return Err(e.into());
    }

    let port = config.port;
    // The API is unauthenticated and exposes destructive endpoints; parse the
    // bind host early so an invalid TURBO_PIX_HOST fails fast before any
    // service is started.
    let host: std::net::IpAddr = config
        .host
        .parse()
        .map_err(|_| format!("Invalid TURBO_PIX_HOST: '{}'", config.host))?;

    // Transcodes write into TRANSCODE_CACHE_DIR (handlers_video reads the env
    // directly); default it under the app data path instead of the
    // world-writable /tmp/turbo-pix so a local user cannot squat the path
    // with a symlink.
    if std::env::var("TRANSCODE_CACHE_DIR").is_err() {
        std::env::set_var(
            "TRANSCODE_CACHE_DIR",
            format!("{}/cache/transcoded", config.data_path),
        );
    }

    // Non-loopback binds with an empty allowlist have NO DNS-rebinding
    // protection (origin==host is trivially satisfiable); warn instead of
    // silently shipping a hardened-but-disabled posture.
    if !host.is_loopback() && config.allowed_hosts.is_empty() {
        log::warn!(
            "Binding non-loopback host {} without TURBO_PIX_ALLOWED_HOSTS — \
             the Host header is not pinned, so a DNS-rebinding page could \
             issue requests as same-origin. Set TURBO_PIX_ALLOWED_HOSTS to \
             the hostnames you access TurboPix from (e.g. my-pix.lan).",
            host
        );
    }

    info!("Starting TurboPix server on Port {}", port);
    info!("Photo paths: {:?}", config.photo_paths);
    info!("Data path: {}", config.data_path);
    info!("Database: {}", config.db_path);
    info!("Cache path: {}", config.cache.thumbnail_cache_path);
    info!("Default locale: {}", config.locale);

    // Check if port is available before initializing services
    if let Some(value) = check_port(&config.host, port) {
        return value;
    }

    // Initialize services
    let (db_pool, thumbnail_generator, photo_scheduler, semantic_search, cache_manager) =
        initialize_services(&config).await?;

    // Extract indexing status before moving photo_scheduler
    let indexing_status = photo_scheduler.status.clone();

    // Start background tasks
    start_background_tasks(photo_scheduler);

    let health_routes = build_health_routes(db_pool.clone());
    let photo_routes = build_photo_routes(
        db_pool.clone(),
        cache_manager,
        config.data_path.clone().into(),
    );
    let thumbnail_routes = build_thumbnail_routes(db_pool.clone(), thumbnail_generator);
    let search_routes = build_search_routes(db_pool.clone(), semantic_search.clone());
    let indexing_routes = build_indexing_routes(indexing_status, db_pool.clone());
    let collage_routes = build_collage_routes(
        db_pool.clone(),
        config.data_path.clone().into(),
        config.locale.clone(),
        semantic_search,
    );
    let housekeeping_routes = build_housekeeping_routes(db_pool.clone());
    let saved_searches_routes = build_saved_searches_routes(db_pool.clone());
    let config_routes = build_config_routes(config.locale.clone());
    let static_routes = build_static_routes();

    // Security posture: reject cross-origin browser requests (no CORS
    // allowance at all). Request bodies are capped at 1 MiB on the JSON
    // routes in build_photo_routes — warp's content_length_limit requires a
    // Content-Length header on every request, so it cannot be global
    // middleware (plain GETs would 411).
    let routes = require_same_origin(&config.allowed_hosts)
        .and(
            health_routes
                .or(photo_routes)
                .or(thumbnail_routes)
                .or(search_routes)
                .or(indexing_routes)
                .or(collage_routes)
                .or(housekeeping_routes)
                .or(saved_searches_routes)
                .or(config_routes)
                .or(static_routes),
        )
        .with(warp::log("turbo_pix"))
        .recover(handle_rejection);

    info!(
        "Server started successfully, listening on http://{}:{}",
        host, port
    );

    warp::serve(routes).run((host, port)).await;

    Ok(())
}

fn check_port(host: &str, port: u16) -> Option<Result<(), Box<dyn Error>>> {
    if TcpListener::bind((host, port)).is_err() {
        error!(
            "Port {} is already in use. Please stop any existing TurboPix instances or use a different port.",
            port
        );
        error!(
            "You can check what's using the port with: lsof -i :{}",
            port
        );
        error!("Or kill the process with: pkill -9 turbo-pix");
        return Some(Err(format!("Port {} is already in use", port).into()));
    }
    None
}

type InitServicesResult = (
    db_pool::DbPool,
    ThumbnailGenerator,
    PhotoScheduler,
    Arc<dyn SemanticSearch>,
    CacheManager,
);

async fn initialize_services(
    config: &config::Config,
) -> Result<InitServicesResult, Box<dyn std::error::Error>> {
    // Initialize database
    let db_pool = db::create_db_pool(&config.db_path).await?;
    info!("Database initialized successfully");

    // Initialize cache manager
    let cache_manager = CacheManager::new(config.cache.thumbnail_cache_path.clone().into());

    // Initialize thumbnail generator
    let thumbnail_generator = ThumbnailGenerator::new(config, db_pool.clone())?;
    info!("Cache and thumbnail system initialized");

    // Initialize semantic search engine
    let semantic_search = Arc::new(
        SemanticSearchEngine::new(db_pool.clone(), &config.data_path)
            .map_err(|e| format!("Failed to initialize semantic search: {}", e))?,
    );
    info!("Semantic search initialized");

    // Initialize and start photo scheduler
    let photo_paths: Vec<PathBuf> = config.photo_paths.iter().map(PathBuf::from).collect();
    let data_path = PathBuf::from(&config.data_path);
    let photo_scheduler = PhotoScheduler::new(
        photo_paths,
        db_pool.clone(),
        cache_manager.clone(),
        semantic_search.clone(),
        data_path,
        config.locale.clone(),
        config.nominatim_url.clone(),
    );
    let _scheduler_handle = photo_scheduler.start();
    info!("Photo scheduler started");

    Ok((
        db_pool,
        thumbnail_generator,
        photo_scheduler,
        semantic_search,
        cache_manager,
    ))
}

fn start_background_tasks(photo_scheduler: PhotoScheduler) {
    info!("Running startup photo rescan and housekeeping...");
    std::thread::Builder::new()
        .name("indexing-startup".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create indexing runtime");
            rt.block_on(async move {
                if let Err(e) = photo_scheduler.run_startup_rescan().await {
                    log::error!("Startup rescan failed: {}", e);
                }
            });
        })
        .expect("Failed to spawn indexing thread");
}
