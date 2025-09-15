use crate::web::handlers::{photos, search, thumbnails};
use actix_web::web;

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/health", web::get().to(photos::health_check))
        .route("/ready", web::get().to(photos::ready_check))
        .route("/metrics", web::get().to(photos::metrics))
        .service(
            web::scope("/api")
                .route("/photos", web::get().to(photos::list_photos))
                .route("/photos", web::post().to(photos::upload_photo))
                .route("/photos/{id}", web::get().to(photos::get_photo))
                .route("/photos/{id}", web::put().to(photos::update_photo))
                .route("/photos/{id}", web::delete().to(photos::delete_photo))
                .route("/photos/{id}/file", web::get().to(photos::get_photo_file))
                .route(
                    "/photos/{id}/thumbnail",
                    web::get().to(photos::get_photo_thumbnail),
                )
                .route(
                    "/photos/{id}/metadata",
                    web::get().to(photos::get_photo_metadata),
                )
                .route("/search", web::get().to(search::search_photos))
                .route(
                    "/search/suggestions",
                    web::get().to(search::search_suggestions),
                )
                .route("/collections", web::get().to(photos::get_collections))
                .route("/cameras", web::get().to(photos::get_cameras))
                .route("/stats", web::get().to(photos::get_stats))
                .route(
                    "/thumbnails/{photo_id}/{size}",
                    web::get().to(thumbnails::get_thumbnail),
                )
                .route("/cache/stats", web::get().to(thumbnails::cache_stats))
                .route("/cache/clear", web::delete().to(thumbnails::clear_cache)),
        );
}
