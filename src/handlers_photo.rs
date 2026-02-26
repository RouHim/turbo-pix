use std::path::Path;

use chrono::{DateTime, Utc};
use image::DynamicImage;
use serde::Deserialize;
use serde_json::json;
use warp::{reject, Filter, Rejection, Reply};

use crate::cache_manager::CacheManager;
use crate::db::{DbPool, Photo, SearchQuery};
use crate::handlers_video::{get_video_file, get_video_status, VideoQuery};
use crate::image_editor::{self, RotationAngle};
use crate::metadata_writer;
use crate::mimetype_detector;
use crate::warp_helpers::{with_cache, with_db, DatabaseError, NotFoundError};

#[derive(Debug, Deserialize)]
pub struct PhotoQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
}

#[derive(Debug, serde::Serialize)]
pub struct PhotosResponse {
    pub photos: Vec<Photo>,
    pub total: usize,
    pub page: u32,
    pub limit: u32,
    pub has_next: bool,
    pub has_prev: bool,
}

pub async fn list_photos(query: PhotoQuery, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = (page - 1) * limit;

    // If a query string or year/month filter is provided, use search instead of list
    let result = if query.q.is_some() || query.year.is_some() || query.month.is_some() {
        let search_query = SearchQuery {
            q: query.q.clone(),
            year: query.year,
            month: query.month,
        };
        Photo::search_photos(
            &db_pool,
            &search_query,
            limit as i64,
            offset as i64,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
        .await
    } else {
        Photo::list_with_pagination(
            &db_pool,
            limit as i64,
            offset as i64,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
        .await
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
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

/// Apply EXIF orientation transformation to an image
/// Matches the orientation values from EXIF specification
fn apply_orientation(img: DynamicImage, orientation: Option<i32>) -> DynamicImage {
    match orientation {
        Some(2) => img.fliph(),
        Some(3) => img.rotate180(),
        Some(4) => img.flipv(),
        Some(5) => img.fliph().rotate270(), // Transpose: flip horizontal, then rotate 90 CCW (270 CW)
        Some(6) => img.rotate90(),
        Some(7) => img.fliph().rotate90(), // Transverse: flip horizontal, then rotate 90 CW
        Some(8) => img.rotate270(),
        _ => img, // 1 or None = no transformation needed
    }
}

pub async fn get_photo(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => Ok(warp::reply::json(&photo)),
        Ok(None) => Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
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
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let file_path = Path::new(&photo.file_path);

    // Check if this is a RAW file that needs conversion
    if crate::raw_processor::is_raw_file(file_path) {
        log::debug!(
            "Converting RAW file to JPEG for detail view: {}",
            photo.file_path
        );

        match crate::raw_processor::decode_raw_to_dynamic_image(file_path) {
            Ok(img) => {
                // Apply orientation correction
                let img = apply_orientation(img, photo.orientation);

                // Encode as JPEG with high quality
                let mut jpeg_data = Vec::new();
                let mut cursor = std::io::Cursor::new(&mut jpeg_data);

                match img.write_to(&mut cursor, image::ImageFormat::Jpeg) {
                    Ok(_) => {
                        let reply =
                            warp::reply::with_header(jpeg_data, "content-type", "image/jpeg");
                        let reply = warp::reply::with_header(
                            reply,
                            "cache-control",
                            "public, max-age=31536000",
                        );
                        return Ok(Box::new(reply));
                    }
                    Err(e) => {
                        log::error!("Failed to encode RAW as JPEG: {}", e);
                        return Err(reject::custom(DatabaseError {
                            message: format!("Failed to encode RAW as JPEG: {}", e),
                        }));
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to decode RAW file {}: {}", photo.file_path, e);
                return Err(reject::custom(DatabaseError {
                    message: format!("Failed to decode RAW file: {}", e),
                }));
            }
        }
    }

    // For non-RAW files, serve directly
    match std::fs::read(&photo.file_path) {
        Ok(file_data) => {
            let content_type = photo.mime_type.unwrap_or_else(|| {
                mimetype_detector::from_path(Path::new(&photo.file_path))
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "application/octet-stream".to_string())
            });

            let reply = warp::reply::with_header(file_data, "content-type", content_type);
            let reply =
                warp::reply::with_header(reply, "cache-control", "public, max-age=31536000");

            Ok(Box::new(reply))
        }
        Err(_) => Err(reject::custom(NotFoundError)),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct FavoriteRequest {
    pub is_favorite: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct MetadataUpdateRequest {
    pub taken_at: Option<String>, // ISO 8601 datetime string
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

pub async fn toggle_favorite(
    photo_hash: String,
    favorite_req: FavoriteRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let mut photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    photo.is_favorite = Some(favorite_req.is_favorite);

    match photo.update(&db_pool).await {
        Ok(_) => Ok(warp::reply::json(&photo)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn update_photo_metadata(
    photo_hash: String,
    metadata_req: MetadataUpdateRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    // Find the photo in database
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    // Parse taken_at if provided
    let taken_at = if let Some(ref dt_str) = metadata_req.taken_at {
        match dt_str.parse::<DateTime<Utc>>() {
            Ok(dt) => Some(dt),
            Err(e) => {
                return Err(reject::custom(DatabaseError {
                    message: format!("Invalid date format: {}", e),
                }));
            }
        }
    } else {
        None
    };

    // Get file path
    let file_path = Path::new(&photo.file_path);

    // Update EXIF in the file
    if let Err(e) = metadata_writer::update_metadata(
        file_path,
        taken_at,
        metadata_req.latitude,
        metadata_req.longitude,
    ) {
        log::error!("Failed to update EXIF: {}", e);
        return Err(reject::custom(DatabaseError {
            message: format!("Failed to update EXIF: {}", e),
        }));
    }

    // Update photo with provided metadata directly
    let mut updated_photo = photo;

    // Update taken_at if provided
    if let Some(dt) = taken_at {
        updated_photo.taken_at = Some(dt);
    }

    // Update GPS coordinates if provided
    if metadata_req.latitude.is_some() || metadata_req.longitude.is_some() {
        let mut location = updated_photo
            .metadata
            .get("location")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        if let Some(lat) = metadata_req.latitude {
            location.insert("latitude".to_string(), json!(lat));
        }
        if let Some(lon) = metadata_req.longitude {
            location.insert("longitude".to_string(), json!(lon));
        }

        updated_photo
            .metadata
            .as_object_mut()
            .unwrap()
            .insert("location".to_string(), json!(location));
    }

    updated_photo.updated_at = Utc::now();

    match updated_photo.update(&db_pool).await {
        Ok(_) => Ok(warp::reply::json(&updated_photo)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_timeline(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::get_timeline_data(&db_pool).await {
        Ok(timeline) => Ok(warp::reply::json(&timeline)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_photo_exif(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    use std::collections::BTreeMap;

    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let file = match std::fs::File::open(&photo.file_path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to open {}: {}", photo.file_path, e);
            return Ok(warp::reply::json(&json!({
                "error": "Failed to open file",
                "message": format!("{}", e)
            })));
        }
    };

    let mut bufreader = std::io::BufReader::new(&file);
    let exifreader = exif::Reader::new();

    let exif_metadata = match exifreader.read_from_container(&mut bufreader) {
        Ok(e) => e,
        Err(e) => {
            log::error!("Failed to read EXIF from {}: {}", photo.file_path, e);
            return Ok(warp::reply::json(&json!({
                "error": "No EXIF data found",
                "message": format!("{}", e)
            })));
        }
    };

    let mut exif_data: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    // Iterate through all fields
    for field in exif_metadata.fields() {
        let tag_name = format!("{}", field.tag);
        let value = field.display_value().to_string();

        exif_data.insert(
            format!("0x{:04X}_{}", field.tag.number(), tag_name),
            json!({
                "value": value,
                "tag": tag_name
            }),
        );
    }

    Ok(warp::reply::json(&json!({
        "hash": photo_hash,
        "filename": photo.filename,
        "exif": exif_data
    })))
}

#[derive(Debug, serde::Deserialize)]
pub struct RotateRequest {
    pub angle: i32, // 90, 180, or 270
}

pub async fn rotate_photo(
    photo_hash: String,
    rotate_req: RotateRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    // Find photo
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    // Parse angle
    let angle = match rotate_req.angle {
        90 => RotationAngle::Rotate90,
        180 => RotationAngle::Rotate180,
        270 => RotationAngle::Rotate270,
        _ => {
            return Err(reject::custom(DatabaseError {
                message: format!(
                    "Invalid rotation angle: {}. Must be 90, 180, or 270",
                    rotate_req.angle
                ),
            }));
        }
    };

    // Rotate image
    match image_editor::rotate_image(&photo, angle, &db_pool).await {
        Ok(updated_photo) => Ok(warp::reply::json(&updated_photo)),
        Err(e) => {
            log::error!("Failed to rotate image: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to rotate image: {}", e),
            }))
        }
    }
}

pub async fn delete_photo(
    photo_hash: String,
    db_pool: DbPool,
    cache_manager: CacheManager,
) -> Result<impl Reply, Rejection> {
    // Find photo
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    // Delete photo
    match image_editor::delete_photo(&photo, &db_pool, &cache_manager).await {
        Ok(()) => Ok(warp::reply::json(
            &json!({"success": true, "message": "Photo deleted successfully"}),
        )),
        Err(e) => {
            log::error!("Failed to delete photo: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to delete photo: {}", e),
            }))
        }
    }
}

pub fn build_photo_routes(
    db_pool: DbPool,
    cache_manager: CacheManager,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let api_photos_list = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<PhotoQuery>())
        .and(with_db(db_pool.clone()))
        .and_then(list_photos);

    let api_photo_get = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_photo);

    let api_photo_file = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("file"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_photo_file);

    let api_photo_video = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("video"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<VideoQuery>())
        .and(warp::header::headers_cloned())
        .and(with_db(db_pool.clone()))
        .and_then(get_video_file);

    let api_photo_video_status = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("video"))
        .and(warp::path("status"))
        .and(warp::path::end())
        .and(warp::get())
        .and_then(get_video_status);

    let api_photo_favorite = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("favorite"))
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::json::<FavoriteRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(toggle_favorite);

    let api_photo_timeline = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("timeline"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_timeline);

    let api_photo_exif = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("exif"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_photo_exif);

    let api_photo_metadata_update = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("metadata"))
        .and(warp::path::end())
        .and(warp::patch())
        .and(warp::body::json::<MetadataUpdateRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(update_photo_metadata);

    let api_photo_rotate = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("rotate"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::json::<RotateRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(rotate_photo);

    let api_photo_delete = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and(with_cache(cache_manager.clone()))
        .and_then(delete_photo);

    api_photos_list
        .or(api_photo_get)
        .or(api_photo_file)
        .or(api_photo_video)
        .or(api_photo_video_status)
        .or(api_photo_favorite)
        .or(api_photo_timeline)
        .or(api_photo_exif)
        .or(api_photo_metadata_update)
        .or(api_photo_rotate)
        .or(api_photo_delete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_in_memory_pool;
    use chrono::{Datelike, TimeZone};
    use std::fs;
    use tempfile::TempDir;

    async fn setup_test_photo(
        db_pool: &DbPool,
        temp_dir: &TempDir,
    ) -> (String, std::path::PathBuf) {
        let test_image = Path::new("test-data/IMG_9377.jpg");
        let temp_image = temp_dir.path().join("test.jpg");
        fs::copy(test_image, &temp_image).expect("Failed to copy test image");

        // Create a test photo in the database
        let photo = Photo {
            hash_sha256: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
            file_path: temp_image.to_str().unwrap().to_string(),
            filename: "test.jpg".to_string(),
            file_size: 12345,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: Some(Utc.with_ymd_and_hms(2020, 1, 1, 12, 0, 0).unwrap()),
            width: Some(800),
            height: Some(600),
            orientation: Some(1),
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: json!({}),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        photo
            .create(db_pool)
            .await
            .expect("Failed to create test photo");

        (photo.hash_sha256.clone(), temp_image)
    }

    #[tokio::test]
    async fn test_update_photo_metadata_endpoint() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");

        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;

        // Create the update request
        let update_req = MetadataUpdateRequest {
            taken_at: Some("2024-03-15T14:30:00Z".to_string()),
            latitude: Some(40.7128),
            longitude: Some(-74.0060),
        };

        // Call the handler
        let result = update_photo_metadata(photo_hash.clone(), update_req, db_pool.clone()).await;

        // Verify the result is ok
        assert!(result.is_ok(), "Handler should succeed");

        // Verify the photo was updated in the database
        let updated_photo = Photo::find_by_hash(&db_pool, &photo_hash)
            .await
            .expect("Failed to query database")
            .expect("Photo should exist");

        // Verify the date was updated
        assert!(updated_photo.taken_at.is_some());
        let taken_at = updated_photo.taken_at.unwrap();
        assert_eq!(taken_at.year(), 2024);
        assert_eq!(taken_at.month(), 3);
        assert_eq!(taken_at.day(), 15);

        // Verify GPS coordinates were updated
        assert_eq!(
            updated_photo
                .metadata
                .get("location")
                .and_then(|l| l.get("latitude"))
                .and_then(|v| v.as_f64()),
            Some(40.7128)
        );
        assert_eq!(
            updated_photo
                .metadata
                .get("location")
                .and_then(|l| l.get("longitude"))
                .and_then(|v| v.as_f64()),
            Some(-74.0060)
        );
    }

    #[tokio::test]
    async fn test_update_photo_metadata_invalid_coordinates() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");

        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;

        // Create request with invalid latitude
        let update_req = MetadataUpdateRequest {
            taken_at: None,
            latitude: Some(91.0), // Invalid: out of range
            longitude: Some(0.0),
        };

        // Call the handler
        let result = update_photo_metadata(photo_hash, update_req, db_pool).await;

        // Verify the result is an error
        assert!(
            result.is_err(),
            "Handler should fail with invalid coordinates"
        );
    }

    #[tokio::test]
    async fn test_update_photo_metadata_missing_longitude() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");

        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;

        // Create request with only latitude (should fail)
        let update_req = MetadataUpdateRequest {
            taken_at: None,
            latitude: Some(40.0),
            longitude: None,
        };

        // Call the handler
        let result = update_photo_metadata(photo_hash, update_req, db_pool).await;

        // Verify the result is an error
        assert!(
            result.is_err(),
            "Handler should fail when GPS coordinates are not paired"
        );
    }
}
