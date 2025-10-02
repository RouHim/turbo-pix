mod cache;
mod config;
mod db;
mod indexer;
mod scheduler;

mod warp_handlers;
mod warp_helpers;

use std::convert::Infallible;
use std::path::PathBuf;
// use std::sync::Arc; // Unused after thumbnail service removal
use log::info;
use warp::Filter;

use cache::{CacheManager, ThumbnailGenerator};
use scheduler::PhotoScheduler;
use warp_helpers::{cors, handle_rejection, with_db, with_thumbnail_generator};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();

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

    // Initialize cache manager
    let cache_manager = CacheManager::new(config.cache.thumbnail_cache_path.clone().into());

    // Initialize thumbnail generator
    let thumbnail_generator = ThumbnailGenerator::new(&config, db_pool.clone())
        .expect("Failed to initialize thumbnail generator");

    info!("Cache and thumbnail system initialized");

    // Start photo scheduler with cache manager
    let photo_paths: Vec<PathBuf> = config.photo_paths.iter().map(PathBuf::from).collect();
    let photo_scheduler = PhotoScheduler::new(photo_paths, db_pool.clone(), cache_manager);
    let _scheduler_handle = photo_scheduler.start();
    info!("Photo scheduler started");

    // Run startup rescan instead of manual scan
    info!("Running startup photo rescan and cleanup...");
    let scheduler_for_rescan = photo_scheduler.clone();
    tokio::spawn(async move {
        if let Err(e) = scheduler_for_rescan.run_startup_rescan().await {
            log::error!("Startup rescan failed: {}", e);
        }
    });

    let host = config.host.clone();
    let port = config.port;

    // Health endpoints
    let health = warp::path("health")
        .and(warp::get())
        .and_then(warp_handlers::health_check);

    let ready = warp::path("ready")
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::ready_check);

    // Photo API endpoints
    let api_photos_list = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<warp_handlers::PhotoQuery>())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::list_photos);

    let api_photo_get = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::get_photo);

    let api_photo_file = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("file"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::get_photo_file);

    let api_photo_video = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("video"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<warp_handlers::VideoQuery>())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::get_video_file);

    let api_photo_favorite = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("favorite"))
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json::<warp_handlers::FavoriteRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::toggle_favorite);

    // Thumbnail endpoints
    let api_photo_thumbnail = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("thumbnail"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<warp_handlers::ThumbnailQuery>())
        .and(with_db(db_pool.clone()))
        .and(with_thumbnail_generator(thumbnail_generator.clone()))
        .and_then(warp_handlers::get_photo_thumbnail);

    let api_thumbnail_by_hash = warp::path("api")
        .and(warp::path("thumbnails"))
        .and(warp::path("hash"))
        .and(warp::path::param::<String>())
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and(with_thumbnail_generator(thumbnail_generator.clone()))
        .and_then(warp_handlers::get_thumbnail_by_hash);

    // Stats endpoints
    let api_stats = warp::path("api")
        .and(warp::path("stats"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::get_stats);

    // Static file serving
    let static_index = warp::path::end().and(warp::get()).and_then(|| async {
        Ok::<_, Infallible>(warp::reply::html(include_str!("../static/index.html")))
    });

    // CSS files
    let static_css_main = warp::path("css")
        .and(warp::path("main.css"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/css/main.css"),
                "content-type",
                "text/css",
            ))
        });

    let static_css_components = warp::path("css")
        .and(warp::path("components.css"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/css/components.css"),
                "content-type",
                "text/css",
            ))
        });

    let static_css_responsive = warp::path("css")
        .and(warp::path("responsive.css"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/css/responsive.css"),
                "content-type",
                "text/css",
            ))
        });

    // JavaScript files
    let static_js_utils = warp::path("js")
        .and(warp::path("utils.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/utils.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_logger = warp::path("js")
        .and(warp::path("logger.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/logger.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_api = warp::path("js")
        .and(warp::path("api.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/api.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_photogrid = warp::path("js")
        .and(warp::path("photoGrid.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/photoGrid.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_viewer = warp::path("js")
        .and(warp::path("viewer.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/viewer.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_search = warp::path("js")
        .and(warp::path("search.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/search.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_i18n = warp::path("js")
        .and(warp::path("i18n.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/i18n.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_app = warp::path("js")
        .and(warp::path("app.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/app.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_feather = warp::path("js")
        .and(warp::path("feather.min.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/feather.min.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_js_icons = warp::path("js")
        .and(warp::path("icons.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/icons.js"),
                "content-type",
                "application/javascript",
            ))
        });

    // i18n files
    let static_i18n_manager = warp::path("i18n")
        .and(warp::path("i18nManager.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/i18n/i18nManager.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_i18n_en = warp::path("i18n")
        .and(warp::path("en"))
        .and(warp::path("index.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/i18n/en/index.js"),
                "content-type",
                "application/javascript",
            ))
        });

    let static_i18n_de = warp::path("i18n")
        .and(warp::path("de"))
        .and(warp::path("index.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/i18n/de/index.js"),
                "content-type",
                "application/javascript",
            ))
        });

    // Combine all routes (API routes first, static files last)
    let api_routes = api_photos_list
        .or(api_photo_get)
        .or(api_photo_file)
        .or(api_photo_video)
        .or(api_photo_favorite)
        .or(api_photo_thumbnail)
        .or(api_thumbnail_by_hash)
        .or(api_stats);

    // Combine static file routes
    let static_routes = static_css_main
        .or(static_css_components)
        .or(static_css_responsive)
        .or(static_js_utils)
        .or(static_js_logger)
        .or(static_js_api)
        .or(static_js_photogrid)
        .or(static_js_viewer)
        .or(static_js_search)
        .or(static_js_i18n)
        .or(static_js_app)
        .or(static_js_feather)
        .or(static_js_icons)
        .or(static_i18n_manager)
        .or(static_i18n_en)
        .or(static_i18n_de)
        .or(static_index);

    let routes = health
        .or(ready)
        .or(api_routes)
        .or(static_routes)
        .with(cors())
        .with(warp::log("turbo_pix"))
        .recover(handle_rejection);

    info!(
        "Server started successfully, listening on {}:{}",
        host, port
    );

    let server = warp::serve(routes).run(([127, 0, 0, 1], port));

    // Start server directly without tokio::select! for now
    server.await;

    Ok(())
}
