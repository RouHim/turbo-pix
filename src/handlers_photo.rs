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
use crate::warp_helpers::{
    with_cache, with_db, DatabaseError, NotFoundError, PermissionError, ValidationError,
};

/// Cap for JSON request bodies (favorite/metadata/rotate). All three payloads
/// are a handful of fields; anything larger is a memory-exhaustion attempt.
const MAX_JSON_BODY_BYTES: u64 = 1024 * 1024;

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

async fn fetch_photos(
    db_pool: &DbPool,
    query: &PhotoQuery,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Photo>, i64), String> {
    if query.q.is_some() || query.year.is_some() || query.month.is_some() {
        let search_query = SearchQuery {
            q: query.q.clone(),
            year: query.year,
            month: query.month,
        };
        Photo::search_photos(
            db_pool,
            &search_query,
            limit,
            offset,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
        .await
        .map_err(|e| format!("{}", e))
    } else {
        Photo::list_with_pagination(
            db_pool,
            limit,
            offset,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
        .await
        .map_err(|e| format!("{}", e))
    }
}

pub async fn list_photos(query: PhotoQuery, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    // Client-supplied pagination must not underflow/overflow: page and limit
    // are clamped to sane ranges before arithmetic.
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let offset = (page as u64 - 1) * limit as u64;

    // Dispatch to helper that selects search vs list
    let result = fetch_photos(&db_pool, &query, limit as i64, offset as i64).await;

    match result {
        Ok((photos, total)) => {
            let has_next = offset.saturating_add(limit as u64) < total as u64;
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

    // For non-RAW files, stream the file instead of buffering it: an
    // unauthenticated client can otherwise force unbounded per-request
    // allocations by requesting many large files concurrently (same pattern
    // as the video route). The explicit content-length keeps hyper from
    // switching to chunked transfer encoding.
    let file = match tokio::fs::File::open(&photo.file_path).await {
        Ok(file) => file,
        Err(_) => return Err(reject::custom(NotFoundError)),
    };
    // Re-stat the open handle so content-length matches the streamed bytes
    // (the file may have been replaced or shrunk since the DB row was read).
    let actual_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let content_type = photo.mime_type.unwrap_or_else(|| {
        mimetype_detector::from_path(Path::new(&photo.file_path))
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string())
    });

    let response = warp::reply::stream(tokio_util::io::ReaderStream::new(file));
    let response = warp::reply::with_header(response, "content-type", content_type);
    let response = warp::reply::with_header(response, "content-length", actual_len.to_string());
    let response = warp::reply::with_header(response, "cache-control", "public, max-age=31536000");
    Ok(Box::new(response))
}

/// Build the response for a HEAD request on a file route: the headers of the
/// corresponding GET route (content-type, content-length, optional
/// accept-ranges, cache-control) but an empty body. Content-length reflects
/// the on-disk file size; no file content is read and no transcoding is
/// triggered. `accept_ranges` is only set for routes whose GET counterpart
/// implements byte ranges (the video route does; the photo-file GET route
/// does not).
fn file_head_reply(
    mime_type: Option<&str>,
    file_path: &Path,
    file_size: i64,
    accept_ranges: bool,
) -> impl Reply {
    let content_type = mime_type
        .map(|m| m.to_string())
        .or_else(|| mimetype_detector::from_path(file_path).map(|m| m.to_string()))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    // Empty body; the explicit content-length mirrors the file size reported by
    // the GET route. The explicit content-length is what makes the HEAD reply
    // useful (e.g. for range planning) without reading any file bytes.
    // Boxed because the accept-ranges header is added conditionally, and warp's
    // typed with_header wrappers would otherwise be two distinct concrete types.
    let reply: Box<dyn Reply> = Box::new(warp::reply::with_header(
        Vec::<u8>::new(),
        "content-type",
        content_type,
    ));
    let reply: Box<dyn Reply> = Box::new(warp::reply::with_header(
        reply,
        "content-length",
        file_size.to_string(),
    ));
    let reply: Box<dyn Reply> = if accept_ranges {
        Box::new(warp::reply::with_header(reply, "accept-ranges", "bytes"))
    } else {
        reply
    };
    Box::new(warp::reply::with_header(
        reply,
        "cache-control",
        "public, max-age=31536000",
    ))
}

pub async fn head_photo_file(photo_hash: String, db_pool: DbPool) -> Result<impl Reply, Rejection> {
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

    // Stat the backing file: content-length must reflect the actual on-disk
    // size (the DB row's file_size can be stale) and a missing file is a 404,
    // not a 200 with a lying size.
    let actual_size = match std::fs::metadata(file_path) {
        Ok(metadata) => metadata.len() as i64,
        Err(_) => return Err(reject::custom(NotFoundError)),
    };

    if crate::raw_processor::is_raw_file(file_path) {
        // The GET route decodes RAW sources to a JPEG on the fly, so HEAD
        // reports content-type: image/jpeg. Content-length is the RAW source
        // size, not the decoded JPEG length: computing the latter would
        // require actually transcoding, which HEAD must not do. This is a
        // documented divergence from what a GET would return.
        return Ok(file_head_reply(
            Some("image/jpeg"),
            file_path,
            actual_size,
            false,
        ));
    }

    Ok(file_head_reply(
        photo.mime_type.as_deref(),
        file_path,
        actual_size,
        false,
    ))
}

pub async fn head_photo_video(
    photo_hash: String,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
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

    // Stat the backing file so a missing file yields 404 and content-length
    // reflects the actual on-disk size. The video GET route implements byte
    // ranges (see handlers_video), so HEAD advertises accept-ranges.
    let actual_size = match std::fs::metadata(&photo.file_path) {
        Ok(metadata) => metadata.len() as i64,
        Err(_) => return Err(reject::custom(NotFoundError)),
    };

    Ok(file_head_reply(
        photo.mime_type.as_deref(),
        Path::new(&photo.file_path),
        actual_size,
        true,
    ))
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
                return Err(reject::custom(ValidationError {
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
        // Out-of-range/unpaired GPS coordinates are client input errors, not
        // server failures — the metadata_writer rejects them before touching
        // the file ("Latitude out of range…", "…without longitude",
        // "…without latitude").
        if e.starts_with("Latitude") || e.starts_with("Longitude") {
            return Err(reject::custom(ValidationError { message: e }));
        }
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

    // GPS coordinates are stored inside the metadata JSON object; make sure the
    // stored value is actually an object before mutating it.
    if !updated_photo.metadata.is_object() {
        updated_photo.metadata = serde_json::json!({});
    }

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
            return Err(reject::custom(NotFoundError));
        }
    };

    let exif_metadata = match crate::exif_helpers::read_exif(&mut std::io::BufReader::new(&file)) {
        Ok(e) => e,
        // A photo without an EXIF APP1/APP2 segment is a normal condition, not
        // a server fault; report 404 so clients can treat it as "no EXIF".
        Err(exif::Error::NotFound(_)) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Failed to read EXIF from {}: {}", photo.file_path, e);
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to read EXIF data: {}", e),
            }));
        }
    };

    fn collect_exif_fields(exif_metadata: &exif::Exif) -> BTreeMap<String, serde_json::Value> {
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

        exif_data
    }

    let exif_data = collect_exif_fields(&exif_metadata);

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

/// Serializes all photo rotations. Two concurrent rotate requests for the
/// same photo would otherwise interleave temp-file writes/renames and DB
/// dimension updates, leaving the stored width/height/hash inconsistent with
/// the actual file (the last rename wins independently of the last DB write).
/// Rotation is a rare user action and the critical section is short.
static ROTATE_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

pub async fn rotate_photo(
    photo_hash: String,
    rotate_req: RotateRequest,
    db_pool: DbPool,
    cache_manager: CacheManager,
) -> Result<impl Reply, Rejection> {
    // Parse angle FIRST (pure input validation, no lock needed)
    let angle = match rotate_req.angle {
        90 => RotationAngle::Rotate90,
        180 => RotationAngle::Rotate180,
        270 => RotationAngle::Rotate270,
        _ => {
            return Err(reject::custom(ValidationError {
                message: format!(
                    "Invalid rotation angle: {}. Must be 90, 180, or 270",
                    rotate_req.angle
                ),
            }));
        }
    };

    // Serialize rotations and re-fetch the photo UNDER the lock: two
    // overlapping rotate requests for the same photo must both read the row
    // after the previous rotation committed, otherwise the second request
    // works from a stale snapshot (double-applied orientation, and its
    // UPDATE ... WHERE hash_sha256 = old_hash matches 0 rows, which
    // update_with_old_hash now rejects loudly). Rotation is a rare user
    // action and the critical section is short.
    let _rotate_guard = ROTATE_LOCK.lock().await;
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

    let old_hash = photo.hash_sha256.clone();
    match image_editor::rotate_image(&photo, angle, &db_pool).await {
        Ok(updated_photo) => {
            // The content hash changed, so all thumbnails under the old hash
            // are stale; remove them so the thumbnail cache cannot grow
            // without bound (they are keyed by hash, see clear_for_hash).
            if let Err(e) = cache_manager.clear_for_hash(&old_hash).await {
                log::warn!("Failed to clear cache for {}: {}", old_hash, e);
            }
            Ok(warp::reply::json(&updated_photo))
        }
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
        Err(image_editor::ImageEditError::PermissionDenied(msg)) => {
            log::warn!("Permission denied deleting photo {}: {}", photo_hash, msg);
            Err(reject::custom(PermissionError { message: msg }))
        }
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

    let api_photo_timeline = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("timeline"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_timeline);

    // NOTE: the literal `/timeline` route must be registered BEFORE the
    // parameterized `api_photo_get` route, otherwise `/api/photos/timeline` is
    // first matched as `get_photo("timeline")`, wasting a database lookup.
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

    let api_photo_file_head = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("file"))
        .and(warp::path::end())
        .and(warp::head())
        .and(with_db(db_pool.clone()))
        .and_then(head_photo_file);

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

    let api_photo_video_head = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("video"))
        .and(warp::path::end())
        .and(warp::head())
        .and(with_db(db_pool.clone()))
        .and_then(head_photo_video);

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
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<FavoriteRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(toggle_favorite);

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
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<MetadataUpdateRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(update_photo_metadata);

    let api_photo_rotate = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("rotate"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<RotateRequest>())
        .and(with_db(db_pool.clone()))
        .and(with_cache(cache_manager.clone()))
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
        .or(api_photo_timeline)
        .or(api_photo_get)
        .or(api_photo_file)
        .or(api_photo_file_head)
        .or(api_photo_video)
        .or(api_photo_video_head)
        .or(api_photo_video_status)
        .or(api_photo_favorite)
        .or(api_photo_exif)
        .or(api_photo_metadata_update)
        .or(api_photo_rotate)
        .or(api_photo_delete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_in_memory_pool;
    use crate::warp_helpers::handle_rejection;
    use chrono::{Datelike, TimeZone};
    use std::convert::Infallible;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Insert a photo row backed by a copied JPEG with the given hash.
    async fn create_photo_row(
        db_pool: &DbPool,
        temp_dir: &TempDir,
        hash: &str,
    ) -> std::path::PathBuf {
        let test_image = Path::new("test-data/IMG_9377.jpg");
        let temp_image = temp_dir.path().join(format!("{}.jpg", hash));
        fs::copy(test_image, &temp_image).expect("Failed to copy test image");

        // Create a test photo in the database
        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: temp_image.to_str().unwrap().to_string(),
            filename: format!("{}.jpg", hash),
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

        temp_image
    }

    async fn setup_test_photo(
        db_pool: &DbPool,
        temp_dir: &TempDir,
    ) -> (String, std::path::PathBuf) {
        let hash = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let temp_image = create_photo_row(db_pool, temp_dir, hash).await;
        (hash.to_string(), temp_image)
    }

    /// Build the full photo route set with rejection handling applied, as the
    /// real server does, so warp::test can exercise the HTTP contract.
    fn build_test_routes(
        db_pool: DbPool,
        cache_dir: PathBuf,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        build_photo_routes(db_pool, CacheManager::new(cache_dir)).recover(handle_rejection)
    }

    #[tokio::test]
    async fn test_update_photo_metadata_endpoint() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;

        let update_req = MetadataUpdateRequest {
            taken_at: Some("2024-03-15T14:30:00Z".to_string()),
            latitude: Some(40.7128),
            longitude: Some(-74.0060),
        };

        let result = update_photo_metadata(photo_hash.clone(), update_req, db_pool.clone()).await;
        assert!(result.is_ok(), "Handler should succeed");

        let updated_photo = Photo::find_by_hash(&db_pool, &photo_hash)
            .await
            .expect("Failed to query database")
            .expect("Photo should exist");

        assert!(updated_photo.taken_at.is_some());
        let taken_at = updated_photo.taken_at.unwrap();
        assert_eq!(taken_at.year(), 2024);
        assert_eq!(taken_at.month(), 3);
        assert_eq!(taken_at.day(), 15);

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

    #[tokio::test]
    async fn test_list_photos_page_zero() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let routes = build_test_routes(db_pool, PathBuf::from("/tmp/turbo-pix-test-cache"));

        // page=0 previously underflowed in `(page - 1)` (debug panic / release
        // wrap); it must be clamped to page 1 and return 200, not 500.
        let response = warp::test::request()
            .path("/api/photos?page=0")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["page"], 1);
        assert_eq!(body["limit"], 50);
        assert_eq!(body["has_prev"], false);
        assert!(body["photos"].is_array());
    }

    #[tokio::test]
    async fn test_list_photos_limit_zero() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let routes = build_test_routes(db_pool, PathBuf::from("/tmp/turbo-pix-test-cache"));

        // limit=0 would otherwise produce a degenerate page; it is clamped to 1.
        let response = warp::test::request()
            .path("/api/photos?limit=0")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["limit"], 1);
        assert_eq!(body["page"], 1);
    }

    #[tokio::test]
    async fn test_timeline_route_not_shadowed() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (_photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        // /api/photos/timeline must be served by the literal timeline route,
        // registered before the parameterized photo-get route (otherwise the
        // request is first matched as get_photo("timeline"), wasting a database
        // lookup before falling through).
        let response = warp::test::request()
            .path("/api/photos/timeline")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert!(body.get("min_date").is_some(), "timeline body has min_date");
        assert!(body.get("max_date").is_some(), "timeline body has max_date");
        assert!(body.get("density").is_some(), "timeline body has density");
        assert!(
            body.get("hash_sha256").is_none(),
            "timeline route must not return photo JSON"
        );
    }

    #[tokio::test]
    async fn test_head_photo_file_returns_headers() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("HEAD")
            .path(&format!("/api/photos/{}/file", photo_hash))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.body().len(),
            0,
            "HEAD responses have an empty body"
        );
        // Content-length must reflect the actual on-disk size (the DB row
        // stores a placeholder 12345), not a stale DB value.
        let actual_size = fs::metadata(&temp_image).unwrap().len().to_string();
        assert_eq!(
            response.headers()["content-length"].to_str().unwrap(),
            actual_size
        );
        assert_eq!(
            response.headers()["content-type"].to_str().unwrap(),
            "image/jpeg"
        );
        assert!(
            response.headers().get("accept-ranges").is_none(),
            "photo-file HEAD must not advertise ranges: the GET route has no Range support"
        );
        assert_eq!(
            response.headers()["cache-control"].to_str().unwrap(),
            "public, max-age=31536000"
        );
    }

    #[tokio::test]
    async fn test_head_photo_file_missing_file_returns_not_found() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        // Remove the backing file: the HEAD handler stats it and must answer
        // 404 rather than a 200 with a stale content-length.
        fs::remove_file(&temp_image).expect("Failed to remove test image");
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("HEAD")
            .path(&format!("/api/photos/{}/file", photo_hash))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn test_head_photo_file_raw_returns_jpeg_content_type() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");

        // Back a photo row with a real RAW source. The DB mime type is the
        // RAW type; the HEAD handler must still advertise image/jpeg because
        // the GET route decodes RAW to JPEG on the fly.
        let hash = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        let raw_source = Path::new("test-data/IMG_9899.CR2");
        let temp_raw = temp_dir.path().join("photo.CR2");
        fs::copy(raw_source, &temp_raw).expect("Failed to copy RAW test image");

        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: temp_raw.to_str().unwrap().to_string(),
            filename: "photo.CR2".to_string(),
            file_size: 12345,
            mime_type: Some("image/x-canon-cr2".to_string()),
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
            .create(&db_pool)
            .await
            .expect("Failed to create test photo");
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("HEAD")
            .path(&format!("/api/photos/{}/file", hash))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"].to_str().unwrap(),
            "image/jpeg",
            "RAW files are served as decoded JPEGs by GET, so HEAD must match"
        );
        // Content-length is the RAW source size (HEAD does not transcode);
        // it is a documented divergence from the decoded GET length.
        let raw_size = fs::metadata(&temp_raw).unwrap().len().to_string();
        assert_eq!(
            response.headers()["content-length"].to_str().unwrap(),
            raw_size
        );
        assert!(
            response.headers().get("accept-ranges").is_none(),
            "photo-file HEAD must not advertise ranges"
        );
    }

    #[tokio::test]
    async fn test_head_photo_video_returns_headers() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("HEAD")
            .path(&format!("/api/photos/{}/video", photo_hash))
            .reply(&routes)
            .await;

        // The video HEAD handler stats the backing file but must not start a
        // transcode.
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.body().len(),
            0,
            "HEAD responses have an empty body"
        );
        let actual_size = fs::metadata(&temp_image).unwrap().len().to_string();
        assert_eq!(
            response.headers()["content-length"].to_str().unwrap(),
            actual_size
        );
        // The video GET route implements byte ranges, so HEAD keeps advertising
        // accept-ranges (unlike the photo-file route).
        assert_eq!(
            response.headers()["accept-ranges"].to_str().unwrap(),
            "bytes"
        );
    }

    #[tokio::test]
    async fn test_head_photo_video_missing_file_returns_not_found() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        fs::remove_file(&temp_image).expect("Failed to remove test image");
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("HEAD")
            .path(&format!("/api/photos/{}/video", photo_hash))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn test_head_not_allowed_on_json_routes() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        // HEAD mirrors exist only for the file/video routes; JSON routes keep
        // returning 405.
        let response = warp::test::request()
            .method("HEAD")
            .path(&format!("/api/photos/{}", photo_hash))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 405);
    }

    #[tokio::test]
    async fn test_rotate_invalid_angle_returns_bad_request() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        // Client-side validation errors (invalid rotation angle) must be 400,
        // not 500.
        let response = warp::test::request()
            .method("POST")
            .path(&format!("/api/photos/{}/rotate", photo_hash))
            .json(&serde_json::json!({ "angle": 45 }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn test_update_metadata_invalid_date_returns_bad_request() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        // An unparseable ISO date is a client error: 400, not 500.
        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", photo_hash))
            .json(&serde_json::json!({ "taken_at": "not-a-date" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn test_invalid_query_param_returns_bad_request() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let routes = build_test_routes(db_pool, PathBuf::from("/tmp/turbo-pix-test-cache"));

        // A malformed query parameter (warp's InvalidQuery rejection) must map
        // to 400 instead of falling through to the generic 500.
        let response = warp::test::request()
            .path("/api/photos?page=abc")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn test_invalid_body_returns_bad_request() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        // A body that cannot be deserialized (warp's BodyDeserializeError) must
        // map to 400 instead of falling through to the generic 500.
        let response = warp::test::request()
            .method("POST")
            .path(&format!("/api/photos/{}/rotate", photo_hash))
            .json(&serde_json::json!({ "angle": "not-a-number" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn test_exif_missing_segment_returns_not_found() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");

        // A plain JPEG with no EXIF APP1 segment: the image crate's encoder
        // writes none. kamadak-exif reports Error::NotFound for it, which must
        // map to 404 ("no EXIF available"), not a 500 server error.
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let temp_image = temp_dir.path().join(format!("{}.jpg", hash));
        image::RgbImage::new(4, 4)
            .save(&temp_image)
            .expect("Failed to write no-EXIF JPEG");

        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: temp_image.to_str().unwrap().to_string(),
            filename: format!("{}.jpg", hash),
            file_size: 12345,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: Some(Utc.with_ymd_and_hms(2020, 1, 1, 12, 0, 0).unwrap()),
            width: Some(4),
            height: Some(4),
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
            .create(&db_pool)
            .await
            .expect("Failed to create test photo");
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .path(&format!("/api/photos/{}/exif", hash))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn test_exif_missing_file_returns_not_found() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        // Remove the backing file so the EXIF handler cannot open it; the route
        // must return 404 (not 200 with an error body).
        fs::remove_file(&temp_image).expect("Failed to remove test image");
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .path(&format!("/api/photos/{}/exif", photo_hash))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 404);
    }
}
