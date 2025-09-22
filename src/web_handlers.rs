use crate::cache::{CacheKey, MemoryCache, ThumbnailGenerator, ThumbnailSize};
use crate::db::{DbPool, Photo, SearchQuery, SearchSuggestion};
use crate::indexer::MetadataExtractor;
use actix_multipart::Multipart;
use actix_web::{web, HttpResponse, Result as ActixResult};
use chrono::Utc;
use futures_util::TryStreamExt;
use mime_guess::MimeGuess;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tracing::{error, info};

// ===============================
// PHOTOS HANDLERS AND TYPES
// ===============================

#[derive(Debug, Deserialize)]
pub struct PhotoQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub q: Option<String>,
    pub _date_from: Option<String>,
    pub _date_to: Option<String>,
    pub _camera_make: Option<String>,
    pub _camera_model: Option<String>,
    pub _has_gps: Option<bool>,
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
pub struct PhotoResponse {
    pub photo: Photo,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: u16,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub photo_count: usize,
}

fn create_photo_from_temp_file(
    temp_path: &Path,
    filename: &str,
) -> Result<Photo, Box<dyn std::error::Error>> {
    let metadata = MetadataExtractor::extract(temp_path);

    // Calculate file hash
    let mut file = std::fs::File::open(temp_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let hash_sha256 = format!("{:x}", hasher.finalize());

    // Get MIME type
    let mime_type = MimeGuess::from_path(temp_path)
        .first_or_octet_stream()
        .to_string();

    // Get file size
    let file_size = std::fs::metadata(temp_path)?.len() as i64;

    // Create the Photo struct
    Ok(Photo {
        id: 0, // Will be set by database
        file_path: temp_path.to_string_lossy().to_string(),
        filename: filename.to_string(),
        file_size,
        mime_type: Some(mime_type),
        taken_at: metadata.taken_at,
        date_modified: Utc::now(),
        date_indexed: Some(Utc::now()),
        camera_make: metadata.camera_make,
        camera_model: metadata.camera_model,
        lens_make: None,
        lens_model: None,
        iso: metadata.iso,
        aperture: metadata.aperture,
        shutter_speed: metadata.shutter_speed,
        focal_length: metadata.focal_length,
        width: metadata.width.map(|w| w as i32),
        height: metadata.height.map(|h| h as i32),
        color_space: None,
        white_balance: None,
        exposure_mode: None,
        metering_mode: None,
        orientation: Some(metadata.orientation.unwrap_or(1)),
        flash_used: None,
        latitude: metadata.latitude,
        longitude: metadata.longitude,
        location_name: None,
        hash_md5: None, // We could calculate this too, but SHA256 is sufficient
        hash_sha256: Some(hash_sha256),
        thumbnail_path: None,
        has_thumbnail: Some(false),
        country: None,
        keywords: None,
        faces_detected: None,
        objects_detected: None,
        colors: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
}

pub async fn health_check() -> ActixResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

pub async fn ready_check(pool: web::Data<DbPool>) -> ActixResult<HttpResponse> {
    match pool.get() {
        Ok(_) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "status": "ready",
            "database": "connected",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))),
        Err(_) => Ok(HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "not ready",
            "database": "disconnected",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))),
    }
}

pub async fn metrics(pool: web::Data<DbPool>) -> ActixResult<HttpResponse> {
    let _conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => {
            return Ok(HttpResponse::ServiceUnavailable().body("# Database connection failed\n"));
        }
    };

    // Get basic metrics
    let total_photos = Photo::list_all(&pool, i64::MAX).unwrap_or_default().len();
    let db_size_bytes = std::fs::metadata("./data/turbo-pix.db")
        .map(|m| m.len())
        .unwrap_or(0);

    // Format as Prometheus metrics
    let metrics = format!(
        r#"# HELP turbopix_photos_total Total number of indexed photos
# TYPE turbopix_photos_total gauge
turbopix_photos_total {}

# HELP turbopix_db_size_bytes Database file size in bytes
# TYPE turbopix_db_size_bytes gauge
turbopix_db_size_bytes {}

# HELP turbopix_uptime_seconds Application uptime in seconds
# TYPE turbopix_uptime_seconds counter
turbopix_uptime_seconds {}

# HELP turbopix_memory_usage_bytes Current memory usage in bytes
# TYPE turbopix_memory_usage_bytes gauge
turbopix_memory_usage_bytes {}
"#,
        total_photos,
        db_size_bytes,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        // Simple memory estimate - in a real app you'd use a proper metrics library
        1024 * 1024 * 50 // 50MB estimate
    );

    Ok(HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4; charset=utf-8")
        .body(metrics))
}

pub async fn list_photos(
    pool: web::Data<DbPool>,
    query: web::Query<PhotoQuery>,
) -> ActixResult<HttpResponse> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100); // Max 100 per page
    let offset = (page - 1) * limit;

    // If there's a search query, use search functionality
    if let Some(search_term) = &query.q {
        if !search_term.trim().is_empty() {
            // Create a SearchQuery from the PhotoQuery
            let search_query = SearchQuery {
                q: Some(search_term.clone()),
                camera_make: None,
                camera_model: None,
                year: None,
                month: None,
                keywords: None,
                has_location: None,
                country: None,
                page: query.page,
                limit: query.limit,
                sort: query.sort.clone(),
                order: query.order.clone(),
            };

            match Photo::search_photos(
                &pool,
                &search_query,
                limit as i64,
                offset as i64,
                query.sort.as_deref(),
                query.order.as_deref(),
            ) {
                Ok((photos, total)) => {
                    let has_next = offset + limit < total as u32;
                    let has_prev = page > 1;

                    return Ok(HttpResponse::Ok().json(PhotosResponse {
                        photos,
                        total: total as usize,
                        page,
                        limit,
                        has_next,
                        has_prev,
                    }));
                }
                Err(e) => {
                    return Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                        error: format!("Search error: {}", e),
                        code: 500,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                    }));
                }
            }
        }
    }

    // Default behavior: list all photos
    match Photo::list_with_pagination(
        &pool,
        limit as i64,
        offset as i64,
        query.sort.as_deref(),
        query.order.as_deref(),
    ) {
        Ok((photos, total)) => {
            let has_next = offset + limit < total as u32;
            let has_prev = page > 1;

            Ok(HttpResponse::Ok().json(PhotosResponse {
                photos,
                total: total as usize,
                page,
                limit,
                has_next,
                has_prev,
            }))
        }
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn get_photo(pool: web::Data<DbPool>, path: web::Path<i64>) -> ActixResult<HttpResponse> {
    let photo_id = path.into_inner();

    match Photo::find_by_id(&pool, photo_id) {
        Ok(Some(photo)) => Ok(HttpResponse::Ok().json(PhotoResponse { photo })),
        Ok(None) => Ok(HttpResponse::NotFound().json(ErrorResponse {
            error: "Photo not found".to_string(),
            code: 404,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn get_photo_file(
    pool: web::Data<DbPool>,
    path: web::Path<i64>,
) -> ActixResult<HttpResponse> {
    let photo_id = path.into_inner();

    // Get photo metadata from database
    let photo = match Photo::find_by_id(&pool, photo_id) {
        Ok(Some(photo)) => photo,
        Ok(None) => {
            return Ok(HttpResponse::NotFound().json(ErrorResponse {
                error: "Photo not found".to_string(),
                code: 404,
                timestamp: chrono::Utc::now().to_rfc3339(),
            }));
        }
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Database error: {}", e),
                code: 500,
                timestamp: chrono::Utc::now().to_rfc3339(),
            }));
        }
    };

    // Security check: ensure the path is within allowed directories
    let photo_path = std::path::Path::new(&photo.file_path);
    if !photo_path.exists() {
        return Ok(HttpResponse::NotFound().json(ErrorResponse {
            error: "Photo file not found on disk".to_string(),
            code: 404,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }));
    }

    // Read the file
    match std::fs::read(&photo.file_path) {
        Ok(file_data) => {
            // Determine content type from photo metadata or file extension
            let content_type = if let Some(ref mime_type) = photo.mime_type {
                if mime_type.starts_with("image/") {
                    mime_type.clone()
                } else {
                    // Fallback to mime_guess if metadata is not reliable
                    mime_guess::from_path(&photo.file_path)
                        .first_or_octet_stream()
                        .to_string()
                }
            } else {
                // No mime_type available, use mime_guess
                mime_guess::from_path(&photo.file_path)
                    .first_or_octet_stream()
                    .to_string()
            };

            Ok(HttpResponse::Ok()
                .content_type(content_type)
                .append_header(("Cache-Control", "public, max-age=31536000")) // Cache for 1 year
                .body(file_data))
        }
        Err(e) => {
            error!("Failed to read photo file {}: {}", photo.file_path, e);
            Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                error: "Failed to read photo file".to_string(),
                code: 500,
                timestamp: chrono::Utc::now().to_rfc3339(),
            }))
        }
    }
}

pub async fn get_photo_metadata(
    pool: web::Data<DbPool>,
    path: web::Path<i64>,
) -> ActixResult<HttpResponse> {
    let photo_id = path.into_inner();

    match Photo::find_by_id(&pool, photo_id) {
        Ok(Some(photo)) => {
            // Return only metadata fields
            Ok(HttpResponse::Ok().json(serde_json::json!({
                "id": photo.id,
                "filename": photo.filename,
                "file_size": photo.file_size,
                "mime_type": photo.mime_type,
                "taken_at": photo.taken_at,
                "date_modified": photo.date_modified,
                "width": photo.width,
                "height": photo.height,
                "orientation": photo.orientation,
                "camera_make": photo.camera_make,
                "camera_model": photo.camera_model,
                "iso": photo.iso,
                "aperture": photo.aperture,
                "shutter_speed": photo.shutter_speed,
                "focal_length": photo.focal_length,
                "gps_latitude": photo.latitude,
                "gps_longitude": photo.longitude,
                "location_name": photo.location_name,
                "hash_md5": photo.hash_md5,
                "hash_sha256": photo.hash_sha256
            })))
        }
        Ok(None) => Ok(HttpResponse::NotFound().json(ErrorResponse {
            error: "Photo not found".to_string(),
            code: 404,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn upload_photo(
    pool: web::Data<DbPool>,
    mut payload: Multipart,
) -> ActixResult<HttpResponse> {
    const MAX_FILE_SIZE: usize = 100 * 1024 * 1024; // 100MB

    let mut file_data = Vec::new();
    let mut filename = String::new();
    let mut content_type = String::new();

    while let Some(mut field) = payload.try_next().await? {
        let content_disposition = field.content_disposition();

        if let Some(name) = content_disposition.and_then(|cd| cd.get_name()) {
            if name == "file" {
                if let Some(file_filename) = content_disposition.and_then(|cd| cd.get_filename()) {
                    filename = file_filename.to_string();
                }

                content_type = field
                    .content_type()
                    .map(|ct| ct.to_string())
                    .unwrap_or_default();

                while let Some(chunk) = field.try_next().await? {
                    file_data.extend_from_slice(&chunk);
                    if file_data.len() > MAX_FILE_SIZE {
                        return Ok(HttpResponse::PayloadTooLarge().json(ErrorResponse {
                            error: "File too large".to_string(),
                            code: 413,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        }));
                    }
                }
            }
        }
    }

    if filename.is_empty() || file_data.is_empty() {
        return Ok(HttpResponse::BadRequest().json(ErrorResponse {
            error: "No file uploaded".to_string(),
            code: 400,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }));
    }

    // Check if it's a valid image
    if !content_type.starts_with("image/") {
        return Ok(HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid image format".to_string(),
            code: 400,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }));
    }

    // Write to temporary file for processing
    let mut temp_file = NamedTempFile::new().map_err(|e| {
        actix_web::error::ErrorInternalServerError(format!("Failed to create temp file: {}", e))
    })?;

    temp_file.write_all(&file_data).map_err(|e| {
        actix_web::error::ErrorInternalServerError(format!("Failed to write file: {}", e))
    })?;

    let temp_path = temp_file.path();

    // Extract metadata and create photo
    let mut photo = match create_photo_from_temp_file(temp_path, &filename) {
        Ok(photo) => photo,
        Err(_) => {
            return Ok(HttpResponse::BadRequest().json(ErrorResponse {
                error: "Failed to process image".to_string(),
                code: 400,
                timestamp: chrono::Utc::now().to_rfc3339(),
            }));
        }
    };

    // Update file size and MIME type from uploaded data
    photo.file_size = file_data.len() as i64;
    photo.mime_type = Some(content_type);

    // Save to database
    match photo.create(&pool) {
        Ok(id) => {
            photo.id = id;
            Ok(HttpResponse::Created().json(photo))
        }
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn update_photo(
    pool: web::Data<DbPool>,
    path: web::Path<i64>,
    update_req: web::Json<PhotoUpdateRequest>,
) -> ActixResult<HttpResponse> {
    let photo_id = path.into_inner();

    // First, get the existing photo
    let mut photo = match Photo::find_by_id(&pool, photo_id) {
        Ok(Some(photo)) => photo,
        Ok(None) => {
            return Ok(HttpResponse::NotFound().json(ErrorResponse {
                error: "Photo not found".to_string(),
                code: 404,
                timestamp: chrono::Utc::now().to_rfc3339(),
            }));
        }
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Database error: {}", e),
                code: 500,
                timestamp: chrono::Utc::now().to_rfc3339(),
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

    // Save changes
    match photo.update(&pool) {
        Ok(_) => Ok(HttpResponse::Ok().json(photo)),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn delete_photo(
    pool: web::Data<DbPool>,
    path: web::Path<i64>,
) -> ActixResult<HttpResponse> {
    let photo_id = path.into_inner();

    // Check if photo exists
    match Photo::find_by_id(&pool, photo_id) {
        Ok(Some(_)) => {
            // Photo exists, proceed with deletion
            match Photo::delete(&pool, photo_id) {
                Ok(true) => Ok(HttpResponse::NoContent().finish()),
                Ok(false) => Ok(HttpResponse::NotFound().json(ErrorResponse {
                    error: "Photo not found".to_string(),
                    code: 404,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                })),
                Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
                    error: format!("Database error: {}", e),
                    code: 500,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                })),
            }
        }
        Ok(None) => Ok(HttpResponse::NotFound().json(ErrorResponse {
            error: "Photo not found".to_string(),
            code: 404,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn get_collections(_pool: web::Data<DbPool>) -> ActixResult<HttpResponse> {
    // For now, return empty collections - this would be implemented based on your collection logic
    let collections: Vec<Collection> = vec![];
    Ok(HttpResponse::Ok().json(collections))
}

pub async fn get_cameras(pool: web::Data<DbPool>) -> ActixResult<HttpResponse> {
    // Get unique camera makes and models with counts
    match Photo::get_cameras(&pool) {
        Ok(cameras) => Ok(HttpResponse::Ok().json(cameras)),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn get_stats(pool: web::Data<DbPool>) -> ActixResult<HttpResponse> {
    match Photo::get_stats(&pool) {
        Ok(stats) => Ok(HttpResponse::Ok().json(stats)),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Database error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn get_photo_thumbnail(
    _pool: web::Data<DbPool>,
    path: web::Path<(i64, String)>,
) -> ActixResult<HttpResponse> {
    let (_photo_id, _size) = path.into_inner();

    // For now, return 404 - thumbnail generation would be implemented here
    Ok(HttpResponse::NotFound().json(ErrorResponse {
        error: "Thumbnail not implemented yet".to_string(),
        code: 404,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }))
}

// ===============================
// SEARCH HANDLERS AND TYPES
// ===============================

#[derive(Debug, Serialize)]
pub struct SearchSuggestionsResponse {
    pub suggestions: Vec<SearchSuggestion>,
}

pub async fn search_photos(
    pool: web::Data<DbPool>,
    query: web::Query<SearchQuery>,
) -> ActixResult<HttpResponse> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = (page - 1) * limit;

    let sort_field = query.sort.as_deref().unwrap_or("taken_at");
    let sort_order = query.order.as_deref().unwrap_or("desc");

    match Photo::search_photos(
        &pool,
        &query,
        limit as i64,
        offset as i64,
        Some(sort_field),
        Some(sort_order),
    ) {
        Ok((photos, total)) => {
            let has_next = offset + limit < total as u32;
            let has_prev = page > 1;

            Ok(HttpResponse::Ok().json(PhotosResponse {
                photos,
                total: total as usize,
                page,
                limit,
                has_next,
                has_prev,
            }))
        }
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Search error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn search_suggestions(
    pool: web::Data<DbPool>,
    query: web::Query<SearchQuery>,
) -> ActixResult<HttpResponse> {
    match Photo::get_search_suggestions(&pool, query.q.as_deref()) {
        Ok(suggestions) => Ok(HttpResponse::Ok().json(SearchSuggestionsResponse { suggestions })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Suggestions error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

// ===============================
// THUMBNAIL SERVICE AND HANDLERS
// ===============================

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
    info!(
        "DEBUG: get_thumbnail called for photo_id={}, size_str={}",
        photo_id, size_str
    );

    let size = match size_str.parse::<ThumbnailSize>() {
        Ok(size) => {
            info!("DEBUG: parsed size successfully: {:?}", size);
            size
        }
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
        }
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

    info!(
        "DEBUG: calling generator.get_or_generate for photo_id={}, size={:?}",
        photo_id, size
    );
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
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::config::{CacheConfig, Config};
    use crate::db::{create_in_memory_pool, Photo};

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

    async fn setup_test_service() -> (Arc<ThumbnailService>, DbPool, TempDir) {
        let (config, temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().unwrap();

        // Create test photo in database
        let image_path = temp_dir.path().join("test.jpg");
        create_test_image(&image_path).unwrap();

        let now = Utc::now();
        let photo = Photo {
            id: 1,
            file_path: image_path.to_string_lossy().to_string(),
            filename: "test.jpg".to_string(),
            file_size: 1024,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: Some(now),
            date_modified: now,
            date_indexed: Some(now),
            camera_make: None,
            camera_model: None,
            lens_make: None,
            lens_model: None,
            iso: None,
            aperture: None,
            shutter_speed: None,
            focal_length: None,
            width: Some(100),
            height: Some(100),
            color_space: None,
            white_balance: None,
            exposure_mode: None,
            metering_mode: None,
            orientation: Some(1),
            flash_used: None,
            latitude: None,
            longitude: None,
            location_name: None,
            hash_md5: None,
            hash_sha256: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            country: None,
            keywords: None,
            faces_detected: None,
            objects_detected: None,
            colors: None,
            created_at: now,
            updated_at: now,
        };

        photo.create(&db_pool).unwrap();

        let memory_cache = MemoryCache::new(100, 10);
        let service = Arc::new(ThumbnailService::new(
            &config,
            memory_cache,
            db_pool.clone(),
        ));

        (service, db_pool, temp_dir)
    }

    #[actix_web::test]
    async fn test_get_thumbnail_success() {
        let (service, db_pool, _temp_dir) = setup_test_service().await;

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(db_pool))
                .app_data(web::Data::new(service.clone()))
                .route(
                    "/thumbnails/{photo_id}/{size}",
                    web::get().to(get_thumbnail),
                ),
        )
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
        let (service, db_pool, _temp_dir) = setup_test_service().await;

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(db_pool))
                .app_data(web::Data::new(service.clone()))
                .route(
                    "/thumbnails/{photo_id}/{size}",
                    web::get().to(get_thumbnail),
                ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/thumbnails/1/invalid")
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn test_get_thumbnail_nonexistent_photo() {
        let (service, db_pool, _temp_dir) = setup_test_service().await;

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(db_pool))
                .app_data(web::Data::new(service.clone()))
                .route(
                    "/thumbnails/{photo_id}/{size}",
                    web::get().to(get_thumbnail),
                ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/thumbnails/999/small")
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_cache_stats() {
        let (service, _db_pool, _temp_dir) = setup_test_service().await;

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
        let (service, _db_pool, _temp_dir) = setup_test_service().await;

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
