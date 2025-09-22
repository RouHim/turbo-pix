use crate::web_handlers;
use actix_web::web;

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/health", web::get().to(web_handlers::health_check))
        .route("/ready", web::get().to(web_handlers::ready_check))
        .route("/metrics", web::get().to(web_handlers::metrics))
        .service(
            web::scope("/api")
                .route("/photos", web::get().to(web_handlers::list_photos))
                .route("/photos", web::post().to(web_handlers::upload_photo))
                .route("/photos/{id}", web::get().to(web_handlers::get_photo))
                .route("/photos/{id}", web::put().to(web_handlers::update_photo))
                .route("/photos/{id}", web::delete().to(web_handlers::delete_photo))
                .route("/photos/{id}/file", web::get().to(web_handlers::get_photo_file))
                .route(
                    "/photos/{id}/thumbnail",
                    web::get().to(web_handlers::get_photo_thumbnail),
                )
                .route(
                    "/photos/{id}/metadata",
                    web::get().to(web_handlers::get_photo_metadata),
                )
                .route("/search", web::get().to(web_handlers::search_photos))
                .route(
                    "/search/suggestions",
                    web::get().to(web_handlers::search_suggestions),
                )
                .route("/collections", web::get().to(web_handlers::get_collections))
                .route("/cameras", web::get().to(web_handlers::get_cameras))
                .route("/stats", web::get().to(web_handlers::get_stats))
                .route(
                    "/thumbnails/{photo_id}/{size}",
                    web::get().to(web_handlers::get_thumbnail),
                )
                .route("/cache/stats", web::get().to(web_handlers::cache_stats))
                .route("/cache/clear", web::delete().to(web_handlers::clear_cache)),
        );
}
