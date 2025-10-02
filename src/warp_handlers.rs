use crate::cache::{ThumbnailGenerator, ThumbnailSize};
use crate::db::{DbPool, Photo, SearchQuery, SearchSuggestion};
use crate::warp_helpers::{DatabaseError, NotFoundError};
use serde::{Deserialize, Serialize};
use serde_json::json;

use std::convert::Infallible;
use std::str::FromStr;
use warp::{reject, Rejection, Reply};

#[derive(Debug, Deserialize)]
pub struct PhotoQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VideoQuery {
    pub metadata: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ThumbnailQuery {
    pub size: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PhotoUpdateRequest {
    pub filename: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub location_name: Option<String>,
    pub is_favorite: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PhotosResponse {
    pub photos: Vec<Photo>,
    pub total: usize,
    pub page: u32,
    pub limit: u32,
    pub has_next: bool,
    pub has_prev: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchSuggestionsResponse {
    pub suggestions: Vec<SearchSuggestion>,
}

pub async fn health_check() -> Result<impl Reply, Infallible> {
    Ok(warp::reply::json(&json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

pub async fn ready_check(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    // Test database connection
    match db_pool.get() {
        Ok(_) => Ok(warp::reply::json(&json!({
            "status": "ready",
            "database": "connected",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))),
        Err(e) => {
            tracing::error!("Database connection failed: {}", e);
            Err(reject::custom(DatabaseError {
                message: "Database connection failed".to_string(),
            }))
        }
    }
}

pub async fn list_photos(query: PhotoQuery, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = (page - 1) * limit;

    // If a query string is provided, use search instead of list
    let result = if let Some(q) = &query.q {
        let search_query = SearchQuery {
            q: Some(q.clone()),
            camera_make: None,
            camera_model: None,
            year: None,
            month: None,
            keywords: None,
            has_location: None,
            country: None,
            limit: Some(limit),
            page: Some(page),
            sort: query.sort.clone(),
            order: query.order.clone(),
        };
        Photo::search_photos(
            &db_pool,
            &search_query,
            limit as i64,
            offset as i64,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
    } else {
        Photo::list_with_pagination(
            &db_pool,
            limit as i64,
            offset as i64,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
    };

    match result {
        Ok((photos, total)) => {
            let has_next = offset + limit < total as u32;
            let has_prev = page > 1;

            Ok(warp::reply::json(&PhotosResponse {
                photos,
                total: total as usize,
                page,
                limit,
                has_next,
                has_prev,
            }))
        }
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_photo(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => Ok(warp::reply::json(&photo)),
        Ok(None) => Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_photo_file(
    photo_hash: String,
    db_pool: DbPool,
) -> Result<Box<dyn Reply>, Rejection> {
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    match std::fs::read(&photo.file_path) {
        Ok(file_data) => {
            let content_type = photo.mime_type.unwrap_or_else(|| {
                mime_guess::from_path(&photo.file_path)
                    .first_or_octet_stream()
                    .to_string()
            });

            let reply = warp::reply::with_header(file_data, "content-type", content_type);
            let reply =
                warp::reply::with_header(reply, "cache-control", "public, max-age=31536000");

            Ok(Box::new(reply))
        }
        Err(_) => Err(reject::custom(NotFoundError)),
    }
}

pub async fn get_video_file(
    photo_hash: String,
    query: VideoQuery,
    db_pool: DbPool,
) -> Result<Box<dyn Reply>, Rejection> {
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let return_metadata_only = query
        .metadata
        .as_ref()
        .map(|v| v == "true")
        .unwrap_or(false);

    if return_metadata_only {
        let video_metadata = json!({
            "hash_sha256": photo.hash_sha256,
            "filename": photo.filename,
            "file_size": photo.file_size,
            "mime_type": photo.mime_type,
            "duration": photo.duration,
            "video_codec": photo.video_codec,
            "audio_codec": photo.audio_codec,
            "bitrate": photo.bitrate,
            "frame_rate": photo.frame_rate,
            "width": photo.width,
            "height": photo.height,
            "taken_at": photo.taken_at.map(|dt| dt.to_rfc3339()),
            "file_path": photo.file_path,
        });

        return Ok(Box::new(warp::reply::json(&video_metadata)));
    }

    match std::fs::read(&photo.file_path) {
        Ok(file_data) => {
            let content_type = photo.mime_type.unwrap_or_else(|| {
                mime_guess::from_path(&photo.file_path)
                    .first_or_octet_stream()
                    .to_string()
            });

            let reply = warp::reply::with_header(file_data, "content-type", content_type);
            let reply =
                warp::reply::with_header(reply, "cache-control", "public, max-age=31536000");
            let reply = warp::reply::with_header(reply, "accept-ranges", "bytes");

            Ok(Box::new(reply))
        }
        Err(_) => Err(reject::custom(NotFoundError)),
    }
}

#[allow(dead_code)]
pub async fn get_photo_metadata(
    photo_hash: String,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => {
            let metadata = json!({
                "hash_sha256": photo.hash_sha256,
                "filename": photo.filename,
                "file_size": photo.file_size,
                "mime_type": photo.mime_type,
                "taken_at": photo.taken_at.map(|dt| dt.to_rfc3339()),
                "date_modified": photo.date_modified.to_rfc3339(),
                "camera_make": photo.camera_make,
                "camera_model": photo.camera_model,
                "iso": photo.iso,
                "aperture": photo.aperture,
                "shutter_speed": photo.shutter_speed,
                "focal_length": photo.focal_length,
                "width": photo.width,
                "height": photo.height,
                "orientation": photo.orientation,
                "gps_latitude": photo.latitude,
                "gps_longitude": photo.longitude,
                "location_name": photo.location_name,
            });
            Ok(warp::reply::json(&metadata))
        }
        Ok(None) => Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn update_photo(
    photo_hash: String,
    update_req: PhotoUpdateRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let mut photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    // Update fields if provided
    if let Some(filename) = &update_req.filename {
        photo.filename = filename.clone();
    }
    if let Some(camera_make) = &update_req.camera_make {
        photo.camera_make = Some(camera_make.clone());
    }
    if let Some(camera_model) = &update_req.camera_model {
        photo.camera_model = Some(camera_model.clone());
    }
    if let Some(iso) = update_req.iso {
        photo.iso = Some(iso);
    }
    if let Some(aperture) = update_req.aperture {
        photo.aperture = Some(aperture);
    }
    if let Some(shutter_speed) = &update_req.shutter_speed {
        photo.shutter_speed = Some(shutter_speed.clone());
    }
    if let Some(focal_length) = update_req.focal_length {
        photo.focal_length = Some(focal_length);
    }
    if let Some(gps_latitude) = update_req.gps_latitude {
        photo.latitude = Some(gps_latitude);
    }
    if let Some(gps_longitude) = update_req.gps_longitude {
        photo.longitude = Some(gps_longitude);
    }
    if let Some(location_name) = &update_req.location_name {
        photo.location_name = Some(location_name.clone());
    }
    if let Some(is_favorite) = update_req.is_favorite {
        photo.is_favorite = Some(is_favorite);
    }

    match photo.update(&db_pool) {
        Ok(_) => Ok(warp::reply::json(&photo)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn delete_photo(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(_)) => match Photo::delete(&db_pool, &photo_hash) {
            Ok(true) => Ok(warp::reply::with_status(
                "",
                warp::http::StatusCode::NO_CONTENT,
            )),
            Ok(false) => Err(reject::custom(NotFoundError)),
            Err(e) => {
                tracing::error!("Database error: {}", e);
                Err(reject::custom(DatabaseError {
                    message: format!("Database error: {}", e),
                }))
            }
        },
        Ok(None) => Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct FavoriteRequest {
    pub is_favorite: bool,
}

pub async fn toggle_favorite(
    photo_hash: String,
    favorite_req: FavoriteRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let mut photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    photo.is_favorite = Some(favorite_req.is_favorite);

    match photo.update(&db_pool) {
        Ok(_) => Ok(warp::reply::json(&photo)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn search_photos(query: SearchQuery, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = (page - 1) * limit;

    let sort_field = query.sort.as_deref().unwrap_or("taken_at");
    let sort_order = query.order.as_deref().unwrap_or("desc");

    match Photo::search_photos(
        &db_pool,
        &query,
        limit as i64,
        offset as i64,
        Some(sort_field),
        Some(sort_order),
    ) {
        Ok((photos, total)) => {
            let has_next = offset + limit < total as u32;
            let has_prev = page > 1;

            Ok(warp::reply::json(&PhotosResponse {
                photos,
                total: total as usize,
                page,
                limit,
                has_next,
                has_prev,
            }))
        }
        Err(e) => {
            tracing::error!("Search error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Search error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn search_suggestions(
    query: SearchQuery,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    match Photo::get_search_suggestions(&db_pool, query.q.as_deref()) {
        Ok(suggestions) => Ok(warp::reply::json(&SearchSuggestionsResponse {
            suggestions,
        })),
        Err(e) => {
            tracing::error!("Suggestions error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Suggestions error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn get_cameras(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::get_cameras(&db_pool) {
        Ok(cameras) => Ok(warp::reply::json(&cameras)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_stats(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::get_stats(&db_pool) {
        Ok(stats) => Ok(warp::reply::json(&stats)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

// Thumbnail endpoints (simplified - fallback to original photo for now)
pub async fn get_photo_thumbnail(
    photo_hash: String,
    query: ThumbnailQuery,
    db_pool: DbPool,
    thumbnail_generator: ThumbnailGenerator,
) -> Result<Box<dyn Reply>, Rejection> {
    tracing::debug!(
        "Thumbnail requested for photo {}, size: {:?}",
        photo_hash,
        query.size
    );

    let photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let size = ThumbnailSize::from_str(&query.size.unwrap_or_else(|| "medium".to_string()))
        .unwrap_or(ThumbnailSize::Medium);

    match thumbnail_generator.get_or_generate(&photo, size).await {
        Ok(thumbnail_data) => {
            let reply = warp::reply::with_header(thumbnail_data, "content-type", "image/jpeg");
            let reply = warp::reply::with_header(
                reply,
                "cache-control",
                "public, max-age=86400", // 24 hours cache for thumbnails
            );

            Ok(Box::new(reply))
        }
        Err(e) => {
            tracing::error!("Failed to generate thumbnail: {}", e);
            Err(reject::custom(NotFoundError))
        }
    }
}

pub async fn get_thumbnail_by_hash(
    hash: String,
    size: String,
    db_pool: DbPool,
    thumbnail_generator: ThumbnailGenerator,
) -> Result<Box<dyn Reply>, Rejection> {
    tracing::debug!("Thumbnail by hash requested: {}, size: {}", hash, size);

    let photo = match Photo::find_by_hash(&db_pool, &hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => {
            tracing::warn!("Photo not found by hash: {}", hash);
            return Err(reject::custom(NotFoundError));
        }
        Err(e) => {
            tracing::error!("Database error looking up photo by hash {}: {}", hash, e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let thumbnail_size = ThumbnailSize::from_str(&size).unwrap_or(ThumbnailSize::Medium);

    match thumbnail_generator
        .get_or_generate(&photo, thumbnail_size)
        .await
    {
        Ok(thumbnail_data) => {
            let reply = warp::reply::with_header(thumbnail_data, "content-type", "image/jpeg");
            let reply = warp::reply::with_header(
                reply,
                "cache-control",
                "public, max-age=86400", // 24 hours cache for thumbnails
            );

            Ok(Box::new(reply))
        }
        Err(e) => {
            tracing::error!("Failed to generate thumbnail for {}: {}", hash, e);
            Err(reject::custom(NotFoundError))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_test_db_pool, Photo};
    use chrono::Utc;

    fn create_test_photo(hash: &str, filename: &str, is_favorite: bool) -> Photo {
        Photo {
            hash_sha256: hash.to_string(),
            file_path: format!("./{}", filename),
            filename: filename.to_string(),
            file_size: 1000,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: Some(Utc::now()),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            camera_make: None,
            camera_model: None,
            lens_make: None,
            lens_model: None,
            iso: None,
            aperture: None,
            shutter_speed: None,
            focal_length: None,
            width: None,
            height: None,
            color_space: None,
            white_balance: None,
            exposure_mode: None,
            metering_mode: None,
            orientation: None,
            flash_used: None,
            latitude: None,
            longitude: None,
            location_name: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            country: None,
            keywords: None,
            faces_detected: None,
            objects_detected: None,
            colors: None,
            duration: None,
            video_codec: None,
            audio_codec: None,
            bitrate: None,
            frame_rate: None,
            is_favorite: Some(is_favorite),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_list_photos_with_favorite_query() {
        // Create test database
        let db_pool = create_test_db_pool().expect("Failed to create test DB");

        // Insert test photos (hash must be 64 chars for SHA256)
        let photo1 = create_test_photo("a".repeat(64).as_str(), "test1.jpg", true);
        let photo2 = create_test_photo("b".repeat(64).as_str(), "test2.jpg", false);

        photo1
            .create_or_update(&db_pool)
            .expect("Failed to insert photo1");
        photo2
            .create_or_update(&db_pool)
            .expect("Failed to insert photo2");

        // Test: Query with is_favorite:true should return only favorited photos
        let search_query = SearchQuery {
            q: Some("is_favorite:true".to_string()),
            camera_make: None,
            camera_model: None,
            year: None,
            month: None,
            keywords: None,
            has_location: None,
            country: None,
            limit: Some(50),
            page: Some(1),
            sort: None,
            order: None,
        };

        let result = Photo::search_photos(&db_pool, &search_query, 50, 0, None, None);
        assert!(result.is_ok());

        let (photos, total) = result.unwrap();
        assert_eq!(total, 1, "Should return only 1 favorite photo");
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].hash_sha256, "a".repeat(64));
        assert_eq!(photos[0].is_favorite, Some(true));
    }

    #[tokio::test]
    async fn test_list_photos_without_query_returns_all() {
        // Create test database
        let db_pool = create_test_db_pool().expect("Failed to create test DB");

        // Insert test photos (hash must be 64 chars for SHA256)
        let photo1 = create_test_photo("c".repeat(64).as_str(), "test3.jpg", true);
        let photo2 = create_test_photo("d".repeat(64).as_str(), "test4.jpg", false);

        photo1
            .create_or_update(&db_pool)
            .expect("Failed to insert photo1");
        photo2
            .create_or_update(&db_pool)
            .expect("Failed to insert photo2");

        // Test: list_with_pagination should return all photos
        let result = Photo::list_with_pagination(&db_pool, 50, 0, None, None);
        assert!(result.is_ok());

        let (photos, total) = result.unwrap();
        assert_eq!(total, 2, "Should return all 2 photos");
        assert_eq!(photos.len(), 2);
    }

    #[tokio::test]
    async fn test_photo_query_with_q_parameter() {
        // Test that PhotoQuery properly includes the q parameter
        let query = PhotoQuery {
            page: Some(1),
            limit: Some(50),
            sort: None,
            order: None,
            q: Some("is_favorite:true".to_string()),
        };

        assert_eq!(query.q, Some("is_favorite:true".to_string()));
    }
}
