mod cache;
mod cache_manager;
mod config;
mod db;
mod db_pool;
mod db_schema;
mod db_types;
mod file_scanner;
mod handlers_health;
mod handlers_photo;
mod handlers_search;
mod handlers_thumbnail;
mod handlers_video;
mod indexer;
mod metadata_extractor;
mod mimetype_detector;
mod photo_processor;
mod scheduler;
mod semantic_search;
mod thumbnail_generator;
mod thumbnail_types;
mod video_processor;
mod warp_handlers;
mod warp_helpers;

use log::{error, info};
use std::convert::Infallible;
use std::net::TcpListener;
use std::path::PathBuf;
use warp::Filter;

use cache::{CacheManager, ThumbnailGenerator};
use scheduler::PhotoScheduler;
use semantic_search::SemanticSearchEngine;
use std::sync::Arc;
use warp_helpers::{
    cors, handle_rejection, with_db, with_semantic_search, with_thumbnail_generator,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let config = config::Config::from_env()?;
    let port = config.port;

    info!("Starting TurboPix server on Port {}", port);
    info!("Photo paths: {:?}", config.photo_paths);
    info!("Data path: {}", config.data_path);
    info!("Database: {}", config.db_path);
    info!("Cache path: {}", config.cache.thumbnail_cache_path);

    // Check if port is available BEFORE initializing services
    if !is_port_available(port) {
        error!(
            "Port {} is already in use. Please stop any existing TurboPix instances or use a different port.",
            port
        );
        error!(
            "You can check what's using the port with: lsof -i :{}",
            port
        );
        error!("Or kill the process with: pkill -9 turbo-pix");
        return Err(format!("Port {} is already in use", port).into());
    }

    let (db_pool, thumbnail_generator, photo_scheduler, semantic_search) =
        initialize_services(&config)?;
    start_background_tasks(photo_scheduler, semantic_search.clone(), db_pool.clone());

    let health_routes = build_health_routes(db_pool.clone());
    let photo_routes = build_photo_routes(db_pool.clone());
    let thumbnail_routes = build_thumbnail_routes(db_pool.clone(), thumbnail_generator);
    let search_routes = build_search_routes(db_pool.clone(), semantic_search);
    let stats_routes = build_stats_routes(db_pool);
    let static_routes = build_static_routes();

    let routes = health_routes
        .or(photo_routes)
        .or(thumbnail_routes)
        .or(search_routes)
        .or(stats_routes)
        .or(static_routes)
        .with(cors())
        .with(warp::log("turbo_pix"))
        .recover(handle_rejection);

    info!(
        "Server started successfully, listening on http://localhost:{}",
        port
    );

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;

    Ok(())
}

fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

type InitServicesResult = (
    db_pool::DbPool,
    ThumbnailGenerator,
    PhotoScheduler,
    Arc<SemanticSearchEngine>,
);

fn initialize_services(
    config: &config::Config,
) -> Result<InitServicesResult, Box<dyn std::error::Error>> {
    let db_pool = db::create_db_pool(&config.db_path)?;
    info!("Database initialized successfully");

    let cache_manager = CacheManager::new(config.cache.thumbnail_cache_path.clone().into());

    let thumbnail_generator = ThumbnailGenerator::new(config, db_pool.clone())?;
    info!("Cache and thumbnail system initialized");

    // Initialize semantic search engine (lazy loading - model loads on first use)
    let semantic_search = Arc::new(
        SemanticSearchEngine::new(db_pool.clone())
            .map_err(|e| format!("Failed to initialize semantic search: {}", e))?,
    );
    info!("Semantic search initialized");

    let photo_paths: Vec<PathBuf> = config.photo_paths.iter().map(PathBuf::from).collect();
    let photo_scheduler = PhotoScheduler::new(
        photo_paths,
        db_pool.clone(),
        cache_manager,
        semantic_search.clone(),
    );
    let _scheduler_handle = photo_scheduler.start();
    info!("Photo scheduler started");

    Ok((
        db_pool,
        thumbnail_generator,
        photo_scheduler,
        semantic_search,
    ))
}

fn start_background_tasks(
    photo_scheduler: PhotoScheduler,
    _semantic_search: Arc<SemanticSearchEngine>,
    _db_pool: db_pool::DbPool,
) {
    info!("Running startup photo rescan and cleanup...");
    tokio::spawn(async move {
        if let Err(e) = photo_scheduler.run_startup_rescan().await {
            log::error!("Startup rescan failed: {}", e);
        }
    });
}

fn build_health_routes(
    db_pool: db_pool::DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let health = warp::path("health")
        .and(warp::get())
        .and_then(warp_handlers::health_check);

    let ready = warp::path("ready")
        .and(warp::get())
        .and(with_db(db_pool))
        .and_then(warp_handlers::ready_check);

    health.or(ready)
}

fn build_photo_routes(
    db_pool: db_pool::DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
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

    let api_photo_timeline = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("timeline"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(warp_handlers::get_timeline);

    let api_photo_exif = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("exif"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool))
        .and_then(warp_handlers::get_photo_exif);

    api_photos_list
        .or(api_photo_get)
        .or(api_photo_file)
        .or(api_photo_video)
        .or(api_photo_favorite)
        .or(api_photo_timeline)
        .or(api_photo_exif)
}

fn build_thumbnail_routes(
    db_pool: db_pool::DbPool,
    thumbnail_generator: ThumbnailGenerator,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
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
        .and(with_db(db_pool))
        .and(with_thumbnail_generator(thumbnail_generator))
        .and_then(warp_handlers::get_thumbnail_by_hash);

    api_photo_thumbnail.or(api_thumbnail_by_hash)
}

fn build_search_routes(
    db_pool: db_pool::DbPool,
    semantic_search: Arc<SemanticSearchEngine>,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("api")
        .and(warp::path("search"))
        .and(warp::path("semantic"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<warp_handlers::SemanticSearchQuery>())
        .and(with_db(db_pool))
        .and(with_semantic_search(semantic_search))
        .and_then(warp_handlers::semantic_search)
}

fn build_stats_routes(
    db_pool: db_pool::DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("api")
        .and(warp::path("stats"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool))
        .and_then(warp_handlers::get_stats)
}

fn build_static_routes() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone
{
    let static_index = warp::path::end().and(warp::get()).and_then(|| async {
        Ok::<_, Infallible>(warp::reply::html(include_str!("../static/index.html")))
    });

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

    let static_js_timeline = warp::path("js")
        .and(warp::path("timeline.js"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(|| async {
            Ok::<_, Infallible>(warp::reply::with_header(
                include_str!("../static/js/timeline.js"),
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

    static_css_main
        .or(static_css_components)
        .or(static_css_responsive)
        .or(static_js_utils)
        .or(static_js_logger)
        .or(static_js_api)
        .or(static_js_photogrid)
        .or(static_js_viewer)
        .or(static_js_search)
        .or(static_js_timeline)
        .or(static_js_i18n)
        .or(static_js_app)
        .or(static_js_feather)
        .or(static_js_icons)
        .or(static_i18n_manager)
        .or(static_i18n_en)
        .or(static_i18n_de)
        .or(static_index)
}
