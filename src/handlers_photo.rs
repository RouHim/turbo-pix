use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
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

/// Default photo page number (1-based) and page size for list responses.
pub(crate) const DEFAULT_PAGE: u32 = 1;
pub(crate) const DEFAULT_PAGE_SIZE: u32 = 50;

/// Hard bounds on client-supplied pagination.
pub(crate) const MIN_PAGE_SIZE: u32 = 1;
pub(crate) const MAX_PAGE_SIZE: u32 = 100;

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
    let page = query.page.unwrap_or(DEFAULT_PAGE).max(DEFAULT_PAGE);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE);
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

    // Check if this is a RAW file that needs conversion. RAW decode + JPEG
    // encode transiently holds several full-resolution buffers (a 45MP
    // sensor can be hundreds of MB per request), so concurrency is capped
    // the same way the transcode path caps ffmpeg jobs — otherwise a handful
    // of concurrent requests exhausts memory.
    if crate::raw_processor::is_raw_file(file_path) {
        log::debug!(
            "Converting RAW file to JPEG for detail view: {}",
            photo.file_path
        );

        let _raw_permit = crate::raw_processor::RAW_DECODE_LIMIT
            .acquire()
            .await
            .map_err(|e| {
                reject::custom(DatabaseError {
                    message: format!("RAW decode queue closed: {}", e),
                })
            })?;

        match crate::raw_processor::decode_raw_to_dynamic_image(file_path) {
            Ok(img) => {
                // Apply orientation correction
                let img = image_editor::apply_orientation(img, photo.orientation);

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
pub struct BatchHashesRequest {
    pub hashes: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchFavoriteRequest {
    pub hashes: Vec<String>,
    pub is_favorite: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct BatchDateShiftRequest {
    pub hashes: Vec<String>,
    pub days: i32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchResult {
    pub applied: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>, // only batch date-shift fills this (photos with no taken_at)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<BatchFailure>,
}

/// Shared batch-size validation for every batch endpoint. An empty array is a
/// client bug; more than 1000 items would let one request pin the server for
/// minutes (each item does its own DB round-trip and possibly file IO).
pub(crate) fn validate_hashes(hashes: &[String]) -> Result<(), Rejection> {
    if hashes.is_empty() {
        return Err(reject::custom(ValidationError {
            message: "hashes must not be empty".to_string(),
        }));
    }
    if hashes.len() > 1000 {
        return Err(reject::custom(ValidationError {
            message: "too many hashes (max 1000)".to_string(),
        }));
    }
    Ok(())
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

/// Batch-delete every selected photo. Partial failure is a 200 with a
/// per-item failure list (FR-011): successful items stay applied and the
/// failures are identified — the request is never rejected wholesale.
pub async fn batch_delete(
    req: BatchHashesRequest,
    db_pool: DbPool,
    cache_manager: CacheManager,
) -> Result<impl Reply, Rejection> {
    validate_hashes(&req.hashes)?;

    let mut result = BatchResult {
        applied: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
    };

    for hash in &req.hashes {
        let photo = match Photo::find_by_hash(&db_pool, hash).await {
            Ok(Some(photo)) => photo,
            Ok(None) => {
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: "Photo not found".to_string(),
                });
                continue;
            }
            Err(e) => {
                log::error!("Database error: {}", e);
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: format!("Database error: {}", e),
                });
                continue;
            }
        };
        match image_editor::delete_photo(&photo, &db_pool, &cache_manager).await {
            Ok(()) => result.applied.push(hash.clone()),
            Err(image_editor::ImageEditError::PermissionDenied(msg)) => {
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: msg,
                });
            }
            Err(e) => {
                log::error!("Failed to delete photo {}: {}", hash, e);
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: e.to_string(),
                });
            }
        }
    }

    Ok(warp::reply::json(&result))
}

/// Batch add/remove favorite. Explicit, never a toggle: mixed favorite
/// states within one selection are resolved by the direction in the request.
pub async fn batch_favorite(
    req: BatchFavoriteRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    validate_hashes(&req.hashes)?;

    let mut result = BatchResult {
        applied: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
    };

    for hash in &req.hashes {
        let mut photo = match Photo::find_by_hash(&db_pool, hash).await {
            Ok(Some(photo)) => photo,
            Ok(None) => {
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: "Photo not found".to_string(),
                });
                continue;
            }
            Err(e) => {
                log::error!("Database error: {}", e);
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: format!("Database error: {}", e),
                });
                continue;
            }
        };
        photo.is_favorite = Some(req.is_favorite);
        match photo.update(&db_pool).await {
            Ok(_) => result.applied.push(hash.clone()),
            Err(e) => {
                log::error!("Database error: {}", e);
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: format!("Database error: {}", e),
                });
            }
        }
    }

    Ok(warp::reply::json(&result))
}

/// Batch date-shift of the taken date by ±N days. Photos without a taken_at
/// are skipped and counted (never silently dropped, never given an invented
/// date); `taken_at` is already a `DateTime<Utc>` after row decode.
pub async fn batch_date_shift(
    req: BatchDateShiftRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    if req.days == 0 {
        return Err(reject::custom(ValidationError {
            message: "days must be non-zero".to_string(),
        }));
    }
    validate_hashes(&req.hashes)?;

    let mut result = BatchResult {
        applied: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
    };

    for hash in &req.hashes {
        let mut photo = match Photo::find_by_hash(&db_pool, hash).await {
            Ok(Some(photo)) => photo,
            Ok(None) => {
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: "Photo not found".to_string(),
                });
                continue;
            }
            Err(e) => {
                log::error!("Database error: {}", e);
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: format!("Database error: {}", e),
                });
                continue;
            }
        };
        match photo.taken_at {
            Some(dt) => {
                photo.taken_at = Some(dt + chrono::Duration::days(req.days as i64));
                match photo.update(&db_pool).await {
                    Ok(_) => result.applied.push(hash.clone()),
                    Err(e) => {
                        log::error!("Database error: {}", e);
                        result.failed.push(BatchFailure {
                            id: hash.clone(),
                            error: format!("Database error: {}", e),
                        });
                    }
                }
            }
            None => result.skipped.push(hash.clone()),
        }
    }

    Ok(warp::reply::json(&result))
}

/// Batch export of the original files as a single ZIP archive. This is the
/// one batch action that can return non-200: when any selected photo is
/// unknown or its backing file is gone, the whole archive cannot be built and
/// the request fails with a 400 carrying the per-item `failed` list (FR-011).
pub async fn batch_export(
    req: BatchHashesRequest,
    db_pool: DbPool,
    data_path: PathBuf,
) -> Result<Box<dyn Reply>, Rejection> {
    validate_hashes(&req.hashes)?;

    // Resolve every photo up front so a missing photo/file fails fast with a
    // JSON body instead of a half-written stream.
    let mut missing = Vec::new();
    let mut photos = Vec::new();
    for hash in &req.hashes {
        match Photo::find_by_hash(&db_pool, hash).await {
            Ok(Some(photo)) => {
                if Path::new(&photo.file_path).exists() {
                    photos.push(photo);
                } else {
                    missing.push(BatchFailure {
                        id: hash.clone(),
                        error: "File not found on disk".to_string(),
                    });
                }
            }
            Ok(None) => missing.push(BatchFailure {
                id: hash.clone(),
                error: "Photo not found".to_string(),
            }),
            Err(e) => {
                log::error!("Database error: {}", e);
                missing.push(BatchFailure {
                    id: hash.clone(),
                    error: format!("Database error: {}", e),
                });
            }
        }
    }

    if !missing.is_empty() {
        let body = warp::reply::json(&serde_json::json!({
            "error": "Some selected photos could not be exported",
            "failed": missing,
        }));
        return Ok(Box::new(warp::reply::with_status(
            body,
            warp::http::StatusCode::BAD_REQUEST,
        )));
    }

    // Build the archive on a blocking thread (zip is sync IO).
    let export_dir = data_path.join("cache").join("export");
    let export_path =
        tokio::task::spawn_blocking(move || build_export_archive(&export_dir, &photos))
            .await
            .map_err(|e| {
                log::error!("Export task panicked: {}", e);
                reject::custom(DatabaseError {
                    message: "Failed to export photos".to_string(),
                })
            })?
            .map_err(|e| {
                log::error!("Failed to build export archive: {}", e);
                reject::custom(DatabaseError {
                    message: "Failed to export photos".to_string(),
                })
            })?;

    let file = match tokio::fs::File::open(&export_path).await {
        Ok(file) => file,
        Err(e) => {
            log::error!("Failed to open export archive: {}", e);
            return Err(reject::custom(DatabaseError {
                message: "Failed to export photos".to_string(),
            }));
        }
    };
    let file_size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let filename = export_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "turbopix-export.zip".to_string());

    let reply = warp::reply::stream(tokio_util::io::ReaderStream::new(file));
    let reply = warp::reply::with_header(reply, "content-type", "application/zip");
    let reply = warp::reply::with_header(reply, "content-length", file_size.to_string());
    let reply = warp::reply::with_header(
        reply,
        "content-disposition",
        format!("attachment; filename=\"{}\"", filename),
    );
    let reply = warp::reply::with_header(reply, "cache-control", "no-store");

    Ok(Box::new(reply))
}

/// Create `turbo-pix-export-{timestamp}.zip` in `export_dir` containing every
/// photo's original file. Stale archives older than 1 hour are swept first
/// (covers crashed/interrupted exports; the sweep can never delete an
/// in-flight archive because that is by definition younger than 1h). Entries
/// use `Stored` compression — photos/videos are already compressed.
/// Duplicate names are disambiguated case-insensitively with `-2`, `-3`, …
/// inserted before the final extension (`IMG_1234-2.CR2`).
fn build_export_archive(export_dir: &Path, photos: &[Photo]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(export_dir)
        .map_err(|e| format!("Failed to create export directory: {}", e))?;

    // Stale sweep: remove `turbo-pix-export-*.zip` older than 1 hour.
    if let Ok(entries) = std::fs::read_dir(export_dir) {
        let now = std::time::SystemTime::now();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("turbo-pix-export-") && name.ends_with(".zip") {
                let stale = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|modified| now.duration_since(modified).ok())
                    .map(|age| age.as_secs() > 3600)
                    .unwrap_or(false);
                if stale {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    // Unique temp name (second resolution; append -n on collision).
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    let mut path = export_dir.join(format!("turbo-pix-export-{}.zip", timestamp));
    let mut n = 2;
    while path.exists() {
        path = export_dir.join(format!("turbo-pix-export-{}-{}.zip", timestamp, n));
        n += 1;
    }

    let file = std::fs::File::create(&path)
        .map_err(|e| format!("Failed to create export archive: {}", e))?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let mut used_names: Vec<String> = Vec::with_capacity(photos.len());
    for photo in photos {
        let base = photo.filename.replace(['/', '\\'], "-");
        let mut name = base.clone();
        let mut suffix = 2;
        while used_names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            name = match base.rfind('.') {
                Some(idx) if idx > 0 => format!("{}-{}{}", &base[..idx], suffix, &base[idx..]),
                _ => format!("{}-{}", base, suffix),
            };
            suffix += 1;
        }
        used_names.push(name.clone());

        zip_writer
            .start_file(name, options)
            .map_err(|e| format!("Failed to write archive entry: {}", e))?;
        let mut source = std::fs::File::open(&photo.file_path)
            .map_err(|e| format!("Failed to open {}: {}", photo.file_path, e))?;
        std::io::copy(&mut source, &mut zip_writer)
            .map_err(|e| format!("Failed to copy {}: {}", photo.file_path, e))?;
    }

    zip_writer
        .finish()
        .map_err(|e| format!("Failed to finalize export archive: {}", e))?;
    Ok(path)
}

pub fn build_photo_routes(
    db_pool: DbPool,
    cache_manager: CacheManager,
    data_path: PathBuf,
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

    // NOTE: the batch literal routes AND the `/timeline` literal route must
    // be registered BEFORE the parameterized `api_photo_get` route. `batch`
    // cannot be swallowed by the param route (`/api/photos/batch/delete` has
    // two extra segments), but keeping the literals first documents the
    // ordering rule in one place.
    let api_photo_batch_delete = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("batch"))
        .and(warp::path("delete"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<BatchHashesRequest>())
        .and(with_db(db_pool.clone()))
        .and(with_cache(cache_manager.clone()))
        .and_then(batch_delete);

    let api_photo_batch_favorite = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("batch"))
        .and(warp::path("favorite"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<BatchFavoriteRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(batch_favorite);

    let api_photo_batch_date_shift = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("batch"))
        .and(warp::path("date-shift"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<BatchDateShiftRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(batch_date_shift);

    let api_photo_batch_export = {
        let data_path = data_path.clone();
        warp::path("api")
            .and(warp::path("photos"))
            .and(warp::path("batch"))
            .and(warp::path("export"))
            .and(warp::path::end())
            .and(warp::post())
            .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
            .and(warp::body::json::<BatchHashesRequest>())
            .and(with_db(db_pool.clone()))
            .map(move |req, db| (req, db, data_path.clone()))
            .untuple_one()
            .and_then(batch_export)
    };
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
        .or(api_photo_batch_delete)
        .or(api_photo_batch_favorite)
        .or(api_photo_batch_date_shift)
        .or(api_photo_batch_export)
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
    /// real server does, so warp::test can exercise the HTTP contract. The
    /// export data path is derived from the cache dir (`{temp}/cache` →
    /// `{temp}/data`) so existing call sites stay untouched.
    fn build_test_routes(
        db_pool: DbPool,
        cache_dir: PathBuf,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        let data_path = cache_dir
            .parent()
            .map(|p| p.join("data"))
            .unwrap_or_else(|| cache_dir.join("data"));
        build_photo_routes(db_pool, CacheManager::new(cache_dir), data_path)
            .recover(handle_rejection)
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

    const BATCH_H1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const BATCH_H2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const BATCH_H3: &str = "3333333333333333333333333333333333333333333333333333333333333333";

    #[tokio::test]
    async fn test_batch_delete_removes_all_selected() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let file1 = create_photo_row(&db_pool, &temp_dir, BATCH_H1).await;
        let file2 = create_photo_row(&db_pool, &temp_dir, BATCH_H2).await;
        let file3 = create_photo_row(&db_pool, &temp_dir, BATCH_H3).await;
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/delete")
            .json(&json!({"hashes": [BATCH_H1, BATCH_H2]}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let result: BatchResult = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(result.applied.len(), 2);
        assert!(result.applied.contains(&BATCH_H1.to_string()));
        assert!(result.applied.contains(&BATCH_H2.to_string()));
        assert!(result.failed.is_empty());
        assert!(!file1.exists());
        assert!(!file2.exists());
        assert!(file3.exists());
        assert!(Photo::find_by_hash(&db_pool, BATCH_H1)
            .await
            .unwrap()
            .is_none());
        assert!(Photo::find_by_hash(&db_pool, BATCH_H2)
            .await
            .unwrap()
            .is_none());
        assert!(Photo::find_by_hash(&db_pool, BATCH_H3)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_batch_delete_reports_missing_hash() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let file1 = create_photo_row(&db_pool, &temp_dir, BATCH_H1).await;
        let file2 = create_photo_row(&db_pool, &temp_dir, BATCH_H2).await;
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let missing = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/delete")
            .json(&json!({"hashes": [BATCH_H1, missing]}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let result: BatchResult = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(result.applied, vec![BATCH_H1.to_string()]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].id, missing);
        assert_eq!(result.failed[0].error, "Photo not found");
        assert!(!file1.exists()); // applied photo file removed
        assert!(file2.exists()); // untouched photo still on disk
    }

    #[tokio::test]
    async fn test_batch_favorite_applies_and_reports_missing() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        create_photo_row(&db_pool, &temp_dir, BATCH_H1).await;
        create_photo_row(&db_pool, &temp_dir, BATCH_H2).await;
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let missing = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/favorite")
            .json(&json!({"hashes": [BATCH_H1, missing], "is_favorite": true}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let result: BatchResult = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(result.applied, vec![BATCH_H1.to_string()]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].id, missing);

        let photo = Photo::find_by_hash(&db_pool, BATCH_H1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(photo.is_favorite, Some(true));
        let photo2 = Photo::find_by_hash(&db_pool, BATCH_H2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(photo2.is_favorite, Some(false));
    }

    #[tokio::test]
    async fn test_batch_date_shift_moves_and_skips() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        create_photo_row(&db_pool, &temp_dir, BATCH_H1).await; // taken_at 2020-01-01
        create_photo_row(&db_pool, &temp_dir, BATCH_H2).await;
        sqlx::query("UPDATE photos SET taken_at = NULL WHERE hash_sha256 = ?")
            .bind(BATCH_H2)
            .execute(&db_pool)
            .await
            .unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/date-shift")
            .json(&json!({"hashes": [BATCH_H1, BATCH_H2], "days": -1}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let result: BatchResult = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(result.applied, vec![BATCH_H1.to_string()]);
        assert_eq!(result.skipped, vec![BATCH_H2.to_string()]);
        assert!(result.failed.is_empty());

        let photo = Photo::find_by_hash(&db_pool, BATCH_H1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            photo.taken_at,
            Some(Utc.with_ymd_and_hms(2019, 12, 31, 12, 0, 0).unwrap())
        );
        assert!(Photo::find_by_hash(&db_pool, BATCH_H2)
            .await
            .unwrap()
            .unwrap()
            .taken_at
            .is_none());
    }

    #[tokio::test]
    async fn test_batch_date_shift_zero_days_rejected() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        create_photo_row(&db_pool, &temp_dir, BATCH_H1).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/date-shift")
            .json(&json!({"hashes": [BATCH_H1], "days": 0}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn test_batch_delete_rejects_empty() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/delete")
            .json(&json!({"hashes": []}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn test_batch_export_zip_entries_disambiguated_and_original_bytes() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        // Two photos with the same filename and one RAW with its own name.
        let jpeg1 = create_photo_row(&db_pool, &temp_dir, BATCH_H1).await;
        let jpeg2 = create_photo_row(&db_pool, &temp_dir, BATCH_H2).await;
        let raw = create_photo_row(&db_pool, &temp_dir, BATCH_H3).await;
        fs::copy("test-data/IMG_9899.CR2", &raw).expect("Failed to copy RAW file");
        sqlx::query("UPDATE photos SET filename = 'same.jpg' WHERE hash_sha256 IN (?, ?)")
            .bind(BATCH_H1)
            .bind(BATCH_H2)
            .execute(&db_pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE photos SET filename = 'IMG_9899.CR2', mime_type = 'image/x-raw' \
             WHERE hash_sha256 = ?",
        )
        .bind(BATCH_H3)
        .execute(&db_pool)
        .await
        .unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/export")
            .json(&json!({"hashes": [BATCH_H1, BATCH_H2, BATCH_H3]}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/zip"
        );
        let disposition = response
            .headers()
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            disposition.contains("turbo-pix-export-"),
            "unexpected disposition: {}",
            disposition
        );
        assert!(disposition.contains(".zip"));

        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(response.body().to_vec()))
            .expect("response body must be a valid ZIP");
        assert_eq!(archive.len(), 3);
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains(&"same.jpg".to_string()));
        assert!(names.contains(&"same-2.jpg".to_string()));
        assert!(names.contains(&"IMG_9899.CR2".to_string()));
        // Names must be pairwise distinct.
        for (i, a) in names.iter().enumerate() {
            for b in names.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }

        // Entry bytes equal the original file bytes (RAW and one JPEG).
        let raw_bytes = {
            let mut raw_entry = archive.by_name("IMG_9899.CR2").unwrap();
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut raw_entry, &mut buf).unwrap();
            buf
        };
        assert_eq!(raw_bytes, fs::read("test-data/IMG_9899.CR2").unwrap());
        let jpeg_bytes = {
            let mut jpeg_entry = archive.by_name("same.jpg").unwrap();
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut jpeg_entry, &mut buf).unwrap();
            buf
        };
        assert_eq!(jpeg_bytes, fs::read(&jpeg1).unwrap());
        // And the disambiguated entry is the second copy.
        let jpeg2_bytes = {
            let mut jpeg2_entry = archive.by_name("same-2.jpg").unwrap();
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut jpeg2_entry, &mut buf).unwrap();
            buf
        };
        assert_eq!(jpeg2_bytes, fs::read(&jpeg2).unwrap());
    }

    #[tokio::test]
    async fn test_batch_export_missing_photo_400() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        create_photo_row(&db_pool, &temp_dir, BATCH_H1).await;
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));
        let missing = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let response = warp::test::request()
            .method("POST")
            .path("/api/photos/batch/export")
            .json(&json!({"hashes": [BATCH_H1, missing]}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["failed"].as_array().unwrap().len(), 1);
        assert_eq!(body["failed"][0]["id"], missing);
    }
}
