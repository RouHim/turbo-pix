use actix_web::{web, HttpResponse, Result as ActixResult};
use std::sync::Arc;
use tracing::{error, info};

use crate::cache::{CacheKey, MemoryCache, ThumbnailGenerator, ThumbnailSize};
use crate::db::{DbPool, Photo};

pub struct ThumbnailService {
    generator: ThumbnailGenerator,
    memory_cache: MemoryCache,
}

impl ThumbnailService {
    pub fn new(config: &crate::config::Config, memory_cache: MemoryCache, db_pool: DbPool) -> Self {
        let generator = ThumbnailGenerator::new(config, db_pool.clone()).unwrap();
        Self {
            generator,
            memory_cache,
        }
    }
}

pub async fn get_thumbnail(
    pool: web::Data<DbPool>,
    path: web::Path<(i64, String)>,
    service: web::Data<Arc<ThumbnailService>>,
) -> ActixResult<HttpResponse> {
    let (photo_id, size_str) = path.into_inner();
    info!("DEBUG: get_thumbnail called for photo_id={}, size_str={}", photo_id, size_str);

    let size = match size_str.parse::<ThumbnailSize>() {
        Ok(size) => {
            info!("DEBUG: parsed size successfully: {:?}", size);
            size
        },
        Err(_) => {
            error!("DEBUG: failed to parse size: {}", size_str);
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Invalid thumbnail size",
                "valid_sizes": ["small", "medium", "large"]
            })));
        }
    };

    let cache_key = CacheKey::new(photo_id, size);
    info!("DEBUG: cache_key created: {}", cache_key);

    // Try memory cache first
    if let Some(data) = service.memory_cache.get(&cache_key) {
        info!("DEBUG: found in memory cache, data length: {}", data.len());
        info!("Serving thumbnail from memory cache: {}", cache_key);
        return Ok(HttpResponse::Ok().content_type("image/jpeg").body(data));
    }
    info!("DEBUG: not found in memory cache");

    // Get photo from database
    let photo = match Photo::find_by_id(&pool, photo_id) {
        Ok(Some(photo)) => {
            info!("DEBUG: found photo in db: path={}", photo.file_path);
            photo
        },
        Ok(None) => {
            error!("DEBUG: photo not found in database: {}", photo_id);
            return Ok(HttpResponse::NotFound().json(serde_json::json!({
                "error": "Photo not found"
            })));
        }
        Err(e) => {
            error!("DEBUG: database error for photo {}: {}", photo_id, e);
            error!("Failed to fetch photo {}: {}", photo_id, e);
            return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to fetch photo"
            })));
        }
    };

    info!("DEBUG: calling generator.get_or_generate for photo_id={}, size={:?}", photo_id, size);
    // Generate or get from disk cache
    match service.generator.get_or_generate(&photo, size).await {
        Ok(data) => {
            info!("DEBUG: generator returned data, length: {}", data.len());
            // Store in memory cache for future requests
            if let Err(e) = service.memory_cache.put(&cache_key, data.clone()) {
                error!("Failed to store thumbnail in memory cache: {}", e);
            }

            info!("DEBUG: preparing HTTP response with {} bytes", data.len());
            info!("Serving generated thumbnail: {}", cache_key);
            Ok(HttpResponse::Ok().content_type("image/jpeg").body(data))
        }
        Err(e) => {
            error!("DEBUG: generator failed: {}", e);
            error!("Failed to generate thumbnail for {}: {}", cache_key, e);
            Ok(HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to generate thumbnail"
            })))
        }
    }
}

pub async fn cache_stats(service: web::Data<Arc<ThumbnailService>>) -> ActixResult<HttpResponse> {
    let (memory_items, memory_capacity, memory_size) = service.memory_cache.stats();
    let (disk_files, disk_size) = service.generator.get_cache_stats().await;
    let hit_rate = service.memory_cache.hit_rate();

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "memory_cache": {
            "items": memory_items,
            "capacity": memory_capacity,
            "size_bytes": memory_size,
            "hit_rate": hit_rate
        },
        "disk_cache": {
            "files": disk_files,
            "size_bytes": disk_size
        }
    })))
}

pub async fn clear_cache(service: web::Data<Arc<ThumbnailService>>) -> ActixResult<HttpResponse> {
    service.memory_cache.clear();

    if let Err(e) = service.generator.clear_cache().await {
        error!("Failed to clear disk cache: {}", e);
        return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "error": "Failed to clear disk cache"
        })));
    }

    info!("Cache cleared successfully");
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "Cache cleared successfully"
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, web, App};
    use chrono::Utc;
    use std::fs::File;
    use std::io::Write;
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::config::{CacheConfig, Config};
    use crate::db::connection::create_in_memory_pool;
    use crate::db::models::Photo;

    fn create_test_config() -> (Config, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("cache");

        let config = Config {
            port: 8080,
            host: "localhost".to_string(),
            photo_paths: vec![],
            db_path: "test.db".to_string(),
            cache_path: cache_path.to_string_lossy().to_string(),
            cache: CacheConfig {
                thumbnail_cache_path: cache_path.join("thumbnails").to_string_lossy().to_string(),
                memory_cache_size: 100,
                memory_cache_max_size_mb: 10,
            },
            thumbnail_sizes: vec![200, 400, 800],
            workers: 1,
            max_connections: 10,
            cache_size_mb: 100,
            scan_interval: 3600,
            batch_size: 1000,
            metrics_enabled: false,
            health_check_path: "/health".to_string(),
        };

        (config, temp_dir)
    }

    fn create_test_image(path: &std::path::Path) -> std::io::Result<()> {
        use image::{ImageBuffer, Rgb};

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Create a simple 10x10 red image
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(10, 10, |_x, _y| {
            Rgb([255, 0, 0]) // Red pixel
        });

        img.save(path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(())
    }

    async fn setup_test_service() -> (Arc<ThumbnailService>, TempDir) {
        let (config, temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().unwrap();

        // Create test photo in database
        let mut conn = db_pool.get().unwrap();
        let image_path = temp_dir.path().join("test.jpg");
        create_test_image(&image_path).unwrap();

        let photo = Photo {
            id: None,
            path: image_path.to_string_lossy().to_string(),
            filename: "test.jpg".to_string(),
            file_size: 1024,
            mime_type: "image/jpeg".to_string(),
            taken_at: Some(Utc::now()),
            date_modified: Utc::now(),
            date_indexed: Utc::now(),
            width: Some(100),
            height: Some(100),
            orientation: 1,
            camera_make: None,
            camera_model: None,
            iso: None,
            aperture: None,
            shutter_speed: None,
            focal_length: None,
            latitude: None,
            longitude: None,
            location_name: None,
            hash_md5: None,
            hash_sha256: None,
            thumbnail_path: None,
            has_thumbnail: false,
        };

        photo.create(&db_pool).unwrap();

        let memory_cache = MemoryCache::new(100, 10);
        let service = Arc::new(ThumbnailService::new(&config, memory_cache, db_pool));

        (service, temp_dir)
    }

    #[actix_web::test]
    async fn test_get_thumbnail_success() {
        let (service, _temp_dir) = setup_test_service().await;

        let app = test::init_service(App::new().app_data(web::Data::new(service.clone())).route(
            "/thumbnails/{photo_id}/{size}",
            web::get().to(get_thumbnail),
        ))
        .await;

        let req = test::TestRequest::get()
            .uri("/thumbnails/1/small")
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        assert_eq!(resp.headers().get("content-type").unwrap(), "image/jpeg");
    }

    #[actix_web::test]
    async fn test_get_thumbnail_invalid_size() {
        let (service, _temp_dir) = setup_test_service().await;

        let app = test::init_service(App::new().app_data(web::Data::new(service.clone())).route(
            "/thumbnails/{photo_id}/{size}",
            web::get().to(get_thumbnail),
        ))
        .await;

        let req = test::TestRequest::get()
            .uri("/thumbnails/1/invalid")
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn test_get_thumbnail_nonexistent_photo() {
        let (service, _temp_dir) = setup_test_service().await;

        let app = test::init_service(App::new().app_data(web::Data::new(service.clone())).route(
            "/thumbnails/{photo_id}/{size}",
            web::get().to(get_thumbnail),
        ))
        .await;

        let req = test::TestRequest::get()
            .uri("/thumbnails/999/small")
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_cache_stats() {
        let (service, _temp_dir) = setup_test_service().await;

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(service.clone()))
                .route("/cache/stats", web::get().to(cache_stats)),
        )
        .await;

        let req = test::TestRequest::get().uri("/cache/stats").to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        let body = test::read_body(resp).await;
        let stats: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(stats["memory_cache"].is_object());
        assert!(stats["disk_cache"].is_object());
        assert!(stats["memory_cache"]["items"].is_number());
        assert!(stats["disk_cache"]["files"].is_number());
    }

    #[actix_web::test]
    async fn test_clear_cache() {
        let (service, _temp_dir) = setup_test_service().await;

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(service.clone()))
                .route("/cache/clear", web::delete().to(clear_cache)),
        )
        .await;

        let req = test::TestRequest::delete().uri("/cache/clear").to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        let body = test::read_body(resp).await;
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(result["message"], "Cache cleared successfully");
    }
}
