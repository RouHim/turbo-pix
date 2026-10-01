use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use warp::{reject, Filter, Rejection, Reply};

use crate::cache_manager::CacheManager;
use crate::db::{DbPool, Photo, SearchQuery};
use crate::handlers_video::{
    get_video_file, get_video_status, stream_video, StreamQuery, VideoQuery,
};
use crate::image_editor::{self, RotationAngle};
use crate::media_facts::MediaFactsIndex;
use crate::metadata_writer;
use crate::mimetype_detector;
use crate::mp4_metadata::{self, Mp4MetadataError, VideoMetadataEdit, VideoMetadataWrite};
use crate::warp_helpers::{
    handle_rejection, with_cache, with_db, with_facts, DatabaseError, NotFoundError,
    PermissionError, ValidationError, VideoMetadataError,
};
use std::sync::Arc;

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
    pub to_year: Option<i32>,
    pub to_month: Option<i32>,
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

/// Query for the Map view listing: the entire filtered photo set, unpaginated
/// (FR-005). `album` scopes to an album's members, mirroring the grid's album
/// detail listing.
#[derive(Debug, Deserialize)]
pub struct MapPhotoQuery {
    pub sort: Option<String>,
    pub order: Option<String>,
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub album: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
pub struct MapPhotosResponse {
    pub photos: Vec<Photo>,
}

async fn fetch_photos(
    db_pool: &DbPool,
    facts: &MediaFactsIndex,
    query: &PhotoQuery,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Photo>, i64), String> {
    if query.q.is_some()
        || query.year.is_some()
        || query.month.is_some()
        || query.to_year.is_some()
        || query.to_month.is_some()
    {
        let search_query = SearchQuery {
            q: query.q.clone(),
            year: query.year,
            month: query.month,
            to_year: query.to_year,
            to_month: query.to_month,
        };
        Photo::search_photos(
            db_pool,
            facts,
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
            facts,
            limit,
            offset,
            query.sort.as_deref(),
            query.order.as_deref(),
        )
        .await
        .map_err(|e| format!("{}", e))
    }
}

pub async fn list_photos(
    query: PhotoQuery,
    db_pool: DbPool,
    facts: Arc<MediaFactsIndex>,
) -> Result<impl Reply, Rejection> {
    // Client-supplied pagination must not underflow/overflow: page and limit
    // are clamped to sane ranges before arithmetic.
    let page = query.page.unwrap_or(DEFAULT_PAGE).max(DEFAULT_PAGE);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE);
    let offset = (page as u64 - 1) * limit as u64;

    // Dispatch to helper that selects search vs list
    let result = fetch_photos(&db_pool, &facts, &query, limit as i64, offset as i64).await;

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

pub async fn list_map_photos(
    query: Option<MapPhotoQuery>,
    db_pool: DbPool,
    facts: Arc<MediaFactsIndex>,
) -> Result<warp::reply::Response, Rejection> {
    // `None` is a query `warp::query` could not deserialize; see the map route.
    let Some(query) = query else {
        // Reuse the shared rejection handler so the 400 body is byte-identical
        // to `/api/photos?page=abc`. `handle_rejection` is infallible, hence the
        // unreachable arm.
        let reply = handle_rejection(reject::custom(ValidationError {
            message: "Invalid query parameters".to_string(),
        }))
        .await
        .unwrap_or_else(|never| match never {});
        return Ok(reply.into_response());
    };

    let search_query = SearchQuery {
        q: query.q.clone(),
        year: query.year,
        month: query.month,
        to_year: None,
        to_month: None,
    };

    match Photo::list_all_filtered(
        &db_pool,
        &facts,
        &search_query,
        query.sort.as_deref(),
        query.order.as_deref(),
        query.album,
    )
    .await
    {
        Ok(photos) => Ok(warp::reply::json(&MapPhotosResponse { photos }).into_response()),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

pub async fn get_photo(
    photo_hash: String,
    db_pool: DbPool,
    facts: Arc<MediaFactsIndex>,
) -> Result<impl Reply, Rejection> {
    match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(mut photo)) => {
            // The date and coordinates live in the file; every response is
            // enriched from the index before it goes out.
            facts.enrich(&mut photo);
            Ok(warp::reply::json(&photo))
        }
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BatchResult {
    pub applied: Vec<String>,
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
    facts: Arc<MediaFactsIndex>,
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
        Ok(_) => {
            // Enrich after the write; the flag is ours, the facts are the
            // file's.
            facts.enrich(&mut photo);
            Ok(warp::reply::json(&photo))
        }
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
    facts: Arc<MediaFactsIndex>,
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

    // Videos carry their metadata in the container, not in EXIF, and a
    // container save is a `moov` rewrite plus a row mirror rather than an
    // EXIF append. The photo path below stays what it always was.
    if photo
        .mime_type
        .as_deref()
        .is_some_and(|m| m.starts_with("video/"))
    {
        // Parsed here rather than in the shared block below, so that an
        // unparsable date can name itself (`invalid_date`) the way the
        // container's own range check does, instead of falling back to the
        // code-less validation error.
        let taken_at = match metadata_req.taken_at.as_deref() {
            Some(dt_str) => match dt_str.parse::<DateTime<Utc>>() {
                Ok(dt) => Some(dt),
                Err(e) => {
                    log::error!("Refused video metadata edit: unparsable date {dt_str:?}: {e}");
                    return Err(video_metadata_rejection(Mp4MetadataError::InvalidDate));
                }
            },
            None => None,
        };

        return apply_video_metadata_edit(
            photo,
            VideoMetadataEdit {
                taken_at,
                latitude: metadata_req.latitude,
                longitude: metadata_req.longitude,
            },
            &db_pool,
            &facts,
        )
        .await;
    }

    // Parse taken_at if provided
    let taken_at = if let Some(dt_str) = &metadata_req.taken_at {
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

    // The file just changed, so re-read it: nothing above the file remembers
    // the new date or coordinates (the row stores neither).
    facts.reload(&photo.file_path);

    // Bookkeeping only: the row keeps its identity and timestamps, but the
    // date and coordinates exist solely in the file.
    let mut updated_photo = photo;
    updated_photo.updated_at = Utc::now();

    match updated_photo.update(&db_pool).await {
        Ok(_) => {
            // Enrich after the DB write: the response must carry what the file
            // now carries, never the requested precision.
            facts.enrich(&mut updated_photo);
            Ok(warp::reply::json(&updated_photo))
        }
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

/// Serializes video metadata saves. A save is a read-modify-write of the
/// container's `moov` region followed by a row write that records the file's
/// new identity; two saves running together would each locate and rewrite the
/// same region from their own read and the second row write would be based on
/// a stale row. Saving a video is a rare user action and the critical section
/// is short, so one lock for all videos is enough (and simpler than a
/// per-file map).
static VIDEO_EDIT_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Classify a container failure into the client-visible refusal. Every variant
/// but [`Mp4MetadataError::Io`] is a condition the client can act on (wrong
/// file kind, no slot for the value, no carrier, read-only medium), so it
/// carries a machine-readable code; an I/O failure is a server fault and takes
/// the shared generic 500 path with no code and a sanitized message.
fn video_metadata_rejection(err: Mp4MetadataError) -> Rejection {
    let (status, code) = match &err {
        Mp4MetadataError::UnsupportedContainer(_) => (
            warp::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_container",
        ),
        Mp4MetadataError::Fragmented | Mp4MetadataError::NoRoom(_) => (
            warp::http::StatusCode::UNPROCESSABLE_ENTITY,
            "no_writable_slot",
        ),
        Mp4MetadataError::NoLocationCarrier => (
            warp::http::StatusCode::UNPROCESSABLE_ENTITY,
            "no_location_carrier",
        ),
        Mp4MetadataError::Unrepresentable(_) => (
            warp::http::StatusCode::UNPROCESSABLE_ENTITY,
            "unrepresentable_value",
        ),
        Mp4MetadataError::InvalidDate => (warp::http::StatusCode::BAD_REQUEST, "invalid_date"),
        Mp4MetadataError::InvalidCoordinates => {
            (warp::http::StatusCode::BAD_REQUEST, "invalid_coordinates")
        }
        Mp4MetadataError::MissingFile => (warp::http::StatusCode::NOT_FOUND, "file_missing"),
        Mp4MetadataError::ReadOnly(_) => (warp::http::StatusCode::FORBIDDEN, "file_read_only"),
        Mp4MetadataError::Io(io) => {
            log::error!("Failed to write video metadata: {}", io);
            return reject::custom(DatabaseError {
                message: format!("Failed to update video metadata: {}", io),
            });
        }
    };

    log::error!("Refused video metadata edit: {} ({} {})", err, status, code);
    reject::custom(VideoMetadataError {
        status,
        code,
        message: err.to_string(),
    })
}

/// The values the container actually holds after a successful write, in the
/// representation a later extraction reads back.
///
/// The carriers keep their own shape: a position is re-rendered in the ISO 6709
/// shape the carrier already had (a fixed decimal count, so a request with more
/// precision is rounded — about 11 m at four decimals), and an instant is whole
/// seconds in the `mvhd`/`tkhd`/`mdhd` fields with text carriers re-rendered at
/// their own fraction width. A field that was not asked for stays `None`, so a
/// save never restates a value the user did not touch.
///
/// A readback that fails keeps the requested value: the write already
/// succeeded, so this is the mirror's reporting step, not a second validation.
/// A readback that succeeds and finds no carrier holding a requested instant is
/// the one answer that is not a report: `Err` says the container cannot
/// represent the value, and the caller rolls the file back and refuses rather
/// than mirroring an instant the file does not carry (a text carrier that
/// cannot be parsed is still a carrier and stays on the reporting path).
fn applied_edit(
    path: &Path,
    requested: VideoMetadataEdit,
) -> Result<VideoMetadataEdit, Mp4MetadataError> {
    let stored = match mp4_metadata::read_metadata(path) {
        Ok(stored) => stored,
        Err(err) => {
            log::warn!(
                "Could not read {} back after a metadata write ({err}); keeping the requested values",
                path.display()
            );
            return Ok(requested);
        }
    };
    // Every carrier the writer renders has been written before this read, so a
    // readback that holds neither the binary time boxes nor a text item with
    // the instant proves the container has no date carrier at all: the write
    // patched nothing date-shaped, and answering the request would claim a date
    // the patched file does not have.
    if requested.taken_at.is_some()
        && stored.creation_time.is_none()
        && stored.creation_date_text.is_none()
    {
        return Err(Mp4MetadataError::Unrepresentable("date"));
    }
    let position = stored
        .location_iso6709
        .as_deref()
        .and_then(mp4_metadata::parse_iso6709);

    Ok(VideoMetadataEdit {
        // The instant the binary time boxes carry — what a later ffprobe
        // reports as `format.tags.creation_time`.
        taken_at: requested
            .taken_at
            .map(|requested| stored.creation_time.unwrap_or(requested)),
        latitude: requested
            .latitude
            .map(|requested| position.map_or(requested, |(latitude, _)| latitude)),
        longitude: requested
            .longitude
            .map(|requested| position.map_or(requested, |(_, longitude)| longitude)),
    })
}

/// True when a save's APPLIED position differs from the one the file held
/// before the save — i.e. when the container's coordinates are actually being
/// moved.
///
/// The row's `location.city` was geocoded from the coordinates the file held,
/// so a save that replaces them invalidates the name and has to re-arm the
/// resolver, which otherwise skips the row forever (its
/// `geo_location_resolved` is already 1) and would pair the old name with the
/// new pin. The position the file held is a file fact, never a stored column,
/// so it comes from the facts index entry captured before the write.
///
/// The APPLIED values decide, not the request: the carriers keep their own
/// representation (16.16 fixed point, four decimals of ISO 6709 — about 11 m
/// at that width), so a request can ask for more precision than the container
/// holds and land on the position the file already has. Comparing the request
/// would report a move that never happened and drop a name that is still the
/// right one, re-queueing a row the resolver would answer with the same
/// string. `applied` is what the file is mirrored from, so this asks the same
/// question that answer creates. A request that carried no pair moves nothing.
fn applied_position_moves_the_file(
    previous: Option<(f64, f64)>,
    applied: &VideoMetadataEdit,
) -> bool {
    let (Some(latitude), Some(longitude)) = (applied.latitude, applied.longitude) else {
        return false;
    };
    // A file that held no position at all is being given one, which is a move.
    previous != Some((latitude, longitude))
}

/// Puts `time` on the file at `path`.
fn set_modified(path: &Path, time: SystemTime) -> Result<(), Mp4MetadataError> {
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(time))
        .map_err(Mp4MetadataError::Io)
}

/// Puts a patched container back through the write's undo token.
///
/// [`mp4_metadata::restore`] refuses unless the file's modification time is the
/// one the token recorded, and that equality rests on `write_moov_region`'s own
/// `set_modified` call, which only WARNS when the kernel refuses it. A save
/// whose clock was not wound back would therefore leave the container patched
/// while the token refuses to undo it — the file and the row permanently
/// disagreeing, the split FR-006 forbids and no later scan repairs, because the
/// row's fingerprint still matches the patched file. So the instant the file
/// carried before the write is put back and the undo retried: the retry
/// succeeding IS the proof that the token recorded that instant, since
/// `restore` accepts nothing else.
///
/// A retry that still refuses means the token names some other instant — the
/// file moved between our read and the write — so the time goes back to the
/// value it was found with (our own repair attempt is not left on the file)
/// and the failure is reported. Nothing here claims a file was repaired that
/// was not.
fn roll_back_container(
    path: &Path,
    write: &VideoMetadataWrite,
    pre_write_modified: Option<SystemTime>,
) -> Result<(), Mp4MetadataError> {
    let found_modified = fs::metadata(path).and_then(|meta| meta.modified()).ok();
    let rearmed = match (pre_write_modified, found_modified) {
        (Some(pre), Some(found)) if pre != found => {
            set_modified(path, pre)?;
            true
        }
        _ => false,
    };
    match mp4_metadata::restore(&write.undo) {
        Ok(()) => Ok(()),
        Err(err) => {
            if rearmed {
                if let Some(found) = found_modified {
                    if let Err(put_back) = set_modified(path, found) {
                        log::error!(
                            "Could not hand the modification time of {} back after a refused rollback: {}",
                            path.display(),
                            put_back
                        );
                    }
                }
            }
            Err(err)
        }
    }
}

/// The container half of a save: the instant the file carried before the write,
/// the write, and the readback of what the file actually holds afterwards.
///
/// Every blocking file operation a save performs lives here, in the order the
/// row write's rollback needs them, so the caller can hand the whole thing to
/// the blocking pool: the file is opened and its `moov` region read twice — up
/// to 64 MiB — and a runtime worker must never sit in that. The edit lock is
/// held by the calling task across the await, so the critical section is
/// exactly as long as it was when these calls were inline.
///
/// A readback that finds no date carrier rolls the file back HERE, before the
/// refusal leaves this function, so no caller can forget the rollback that
/// keeps a refused save from leaving a patched file behind.
fn patch_container(
    path: &Path,
    requested: VideoMetadataEdit,
) -> Result<(Option<SystemTime>, VideoMetadataWrite, VideoMetadataEdit), Rejection> {
    // The undo token's guard is the file's modification time, so the rollbacks
    // need the instant the file carried before the write.
    let pre_write_modified = fs::metadata(path).and_then(|meta| meta.modified()).ok();

    let write = match mp4_metadata::write_metadata(path, &requested) {
        Ok(write) => write,
        Err(err) => return Err(video_metadata_rejection(err)),
    };

    // The container keeps the carrier's own representation, not the request's:
    // mirroring the request would leave the row (and this response) claiming
    // precision the file does not hold, and `metadata_extractor` re-derives
    // both fields from those carriers — so the next scan of a changed file
    // would silently move the stored values by the rounding error. Read back
    // what the write actually left.
    let applied = match applied_edit(path, requested) {
        Ok(applied) => applied,
        Err(err) => {
            // The readback proved the container holds no carrier for the
            // requested instant. Put the file back (the write may still have
            // patched a location carrier) so the refusal leaves the file and
            // the row in agreement.
            if let Err(rollback) = roll_back_container(path, &write, pre_write_modified) {
                log::error!(
                    "Could not roll back {} after refusing a date with no carrier: {}",
                    path.display(),
                    rollback
                );
                return Err(reject::custom(DatabaseError {
                    message: format!(
                        "Video metadata rollback failed for {}: {}",
                        path.display(),
                        rollback
                    ),
                }));
            }
            return Err(video_metadata_rejection(err));
        }
    };

    Ok((pre_write_modified, write, applied))
}

/// Apply a metadata edit to a video's container and record the row-side facts
/// of the save.
///
/// The container is the source of truth: the file is rewritten first (every
/// refusal [`mp4_metadata::write_metadata`] can decide is decided before a byte
/// is written), and the row then records only what the file cannot: its
/// `updated_at`, the scanner's identity of the patched file (only when the row
/// already described that file), and — when the save moved the position — the
/// derived place name and the resolver gate. The date and the coordinates are
/// NOT written to the row: they are file facts, served from
/// [`MediaFactsIndex`], which is reloaded here so the response carries what the
/// container now holds. A container that turns out to hold no date carrier is
/// refused right after the write, with the file rolled back first. When the row
/// write fails, the file is rolled back through the write's undo token, so a
/// 500 never leaves the file changed without the row that describes it.
///
/// A rollback that could not run is never the same answer as a clean one: the
/// 422 becomes a 500 naming the failed rollback, because "nothing was written"
/// over a container that still holds the values is the one answer a client
/// cannot recover from.
async fn apply_video_metadata_edit(
    photo: Photo,
    edit: VideoMetadataEdit,
    db_pool: &DbPool,
    facts: &MediaFactsIndex,
) -> Result<warp::reply::Json, Rejection> {
    // FR-013: an empty request must not rewrite the container. Nothing was
    // asked for, so the row comes back as it was read.
    if edit.taken_at.is_none() && edit.latitude.is_none() && edit.longitude.is_none() {
        log::debug!(
            "Empty metadata request for video {}; file and row left untouched",
            photo.hash_sha256
        );
        return Ok(warp::reply::json(&photo));
    }

    let _guard = VIDEO_EDIT_LOCK.lock().await;

    // The row read before this lock is a pre-save snapshot: merging into it
    // would drop a field a save that just finished had written. Re-read it
    // under the lock so the mirror is a merge into committed state.
    let mut photo = match Photo::find_by_hash(db_pool, &photo.hash_sha256).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    // The position the file held before the write is a file fact (the DB stores
    // no coordinates), so capture it from the index now: the patch below
    // replaces whatever the container carries.
    let previous_position =
        facts
            .get(&photo.file_path)
            .and_then(|facts| match (facts.latitude, facts.longitude) {
                (Some(latitude), Some(longitude)) => Some((latitude, longitude)),
                _ => None,
            });

    // The container is patched on the blocking pool for the reason
    // `patch_container` documents. This task keeps holding the edit lock across
    // the await, so the critical section is unchanged.
    let (pre_write_modified, write, applied) = tokio::task::spawn_blocking({
        let path = PathBuf::from(&photo.file_path);
        move || patch_container(&path, edit)
    })
    .await
    .map_err(|error| {
        log::error!("Video metadata save task panicked: {}", error);
        reject::custom(DatabaseError {
            message: "Failed to update video metadata".to_string(),
        })
    })??;

    // The save owns the position the container now holds and nothing else: the
    // stored document also holds members no file carries — a resolved place
    // name, which belongs to the resolver and only leaves the row when the
    // position it was derived from is the one being replaced.
    let position_moved = applied_position_moves_the_file(previous_position, &applied);
    if position_moved {
        // Drop it from the response copy too, so the caller is handed the state
        // that was committed instead of a new pin next to the old place name.
        if let Some(location) = photo
            .metadata
            .get_mut("location")
            .and_then(|v| v.as_object_mut())
        {
            location.remove("city");
        }
    }
    // Record the patched file's identity only when the row already describes
    // that file. The rewrite preserves the byte length and the modification
    // time, so `write.fingerprint` is the identity of the file at this path
    // before the patch as well as after it: a row whose stored fingerprint
    // equals it describes exactly the file that was patched, and restating it
    // keeps the row "unchanged" for `find_unchanged_photo`. A row that
    // disagrees describes a DIFFERENT file — the video at this path was
    // replaced since the last scan (a re-export, a copy from another tool) —
    // and stamping the new bytes' identity here would erase the mismatch the
    // change detection keys on: every later scan would skip the file while the
    // row kept the previous file's facts (`metadata.video.*`, width/height/
    // duration/orientation). The stale fingerprint stays, so the next scan
    // re-extracts the file; the save's values live in the container, which is
    // where that extraction re-derives them from.
    // The equal case is a no-op by construction — both assignments restate what
    // the condition just proved — so the branch exists to carry that asymmetry,
    // not to protect the row: DO NOT collapse it into an unconditional stamp,
    // which would erase the mismatch the change detection keys on and let
    // every later scan skip a file whose row still describes the previous one.
    let row_describes_patched_file = photo.file_size == write.fingerprint.file_size as i64
        && photo.date_modified == write.fingerprint.file_modified;
    if row_describes_patched_file {
        photo.file_size = write.fingerprint.file_size as i64;
        photo.date_modified = write.fingerprint.file_modified;
    } else {
        log::debug!(
            "Row for {} still describes the file it was scanned from; keeping its fingerprint so the next scan re-extracts {}",
            photo.hash_sha256,
            photo.file_path
        );
    }
    photo.updated_at = Utc::now();

    // Only what the save wrote: the row may have taken a commit since it was
    // re-read, and a replacement of the whole document would revert it.
    let identity = row_describes_patched_file.then_some((
        write.fingerprint.file_size as i64,
        write.fingerprint.file_modified,
    ));

    // The row write's error is a boxed, non-`Send` value, and a `match`
    // scrutinee lives until the end of the match — so render it into a message
    // here, before the rollback's await below can hold this handler's future
    // across a non-`Send` value, which warp's `and_then` refuses to build a
    // route from.
    let row_write = Photo::record_video_edit(db_pool, &photo.hash_sha256, position_moved, identity)
        .await
        .map_err(|error| error.to_string());

    match row_write {
        Ok(()) => {
            // The container just changed and the row stores neither the date
            // nor the coordinates: re-read the file's facts so the response
            // carries what it now holds.
            facts.reload(&photo.file_path);
            facts.enrich(&mut photo);
            Ok(warp::reply::json(&photo))
        }
        Err(db_error) => {
            log::error!("Database error after a video metadata write: {}", db_error);
            // The container is already rewritten; putting it back keeps the
            // file and the row in agreement. That rollback is container I/O
            // like the patch above, so it runs on the same pool.
            let file_path = PathBuf::from(&photo.file_path);
            let rollback = tokio::task::spawn_blocking(move || {
                roll_back_container(&file_path, &write, pre_write_modified)
            })
            .await;
            let message = match rollback {
                Ok(Ok(())) => format!("Database error: {}", db_error),
                Ok(Err(rollback)) => {
                    log::error!(
                        "Could not roll back {} after a failed row write: {}",
                        photo.file_path,
                        rollback
                    );
                    format!(
                        "Database error: {}; the container could not be put back ({})",
                        db_error, rollback
                    )
                }
                Err(error) => {
                    log::error!("Video metadata rollback task panicked: {}", error);
                    format!(
                        "Database error: {}; the container could not be put back ({})",
                        db_error, error
                    )
                }
            };
            Err(reject::custom(DatabaseError { message }))
        }
    }
}

pub async fn get_timeline(
    db_pool: DbPool,
    facts: Arc<MediaFactsIndex>,
) -> Result<impl Reply, Rejection> {
    match Photo::get_timeline_data(&db_pool, &facts).await {
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
    facts: Arc<MediaFactsIndex>,
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
        Ok(mut updated_photo) => {
            // The content hash changed, so all thumbnails under the old hash
            // are stale; remove them so the thumbnail cache cannot grow
            // without bound (they are keyed by hash, see clear_for_hash).
            if let Err(e) = cache_manager.clear_for_hash(&old_hash).await {
                log::warn!("Failed to clear cache for {}: {}", old_hash, e);
            }
            // Same staleness rule as thumbnails: the content version changed, so the
            // old hash's conversions can never be served again.
            crate::video_processor::clear_transcode_cache_for_hash(&old_hash);
            // The row write is done; the reply carries the file's facts (the
            // index is keyed by path, which survives the hash re-key).
            facts.enrich(&mut updated_photo);
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
    facts: Arc<MediaFactsIndex>,
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
    match image_editor::delete_photo(&photo, &db_pool, &cache_manager, &facts).await {
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
    facts: Arc<MediaFactsIndex>,
) -> Result<impl Reply, Rejection> {
    validate_hashes(&req.hashes)?;

    let mut result = BatchResult {
        applied: Vec::new(),
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
        match image_editor::delete_photo(&photo, &db_pool, &cache_manager, &facts).await {
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
    media_facts: Arc<MediaFactsIndex>,
    cache_manager: CacheManager,
    data_path: PathBuf,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let api_photos_list = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<PhotoQuery>())
        .and(with_db(db_pool.clone()))
        .and(with_facts(media_facts.clone()))
        .and_then(list_photos);

    let api_photo_timeline = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("timeline"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and(with_facts(media_facts.clone()))
        .and_then(get_timeline);

    // Literal sub-paths (`/map`, `/timeline`, `/batch/...`) must be registered
    // BEFORE the parameterized `api_photo_get` route, otherwise `map` would be
    // captured as a photo hash (same rule as the NOTE below).
    let api_photos_map = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path("map"))
        .and(warp::path::end())
        .and(warp::get())
        // A malformed parameter (`?year=abc`) makes `warp::query` reject with
        // `InvalidQuery`, but that rejection arrives *together* with the
        // `{hash}` route's `NotFoundError` (it matches `/api/photos/map` as
        // well) and `handle_rejection` tests `NotFoundError` first — answering
        // 404 "Photo not found" instead of the project's 400. Extract an
        // `Option` so the map route answers the malformed case itself.
        .and(
            warp::query::<MapPhotoQuery>()
                .map(Some)
                .or(warp::any().map(|| None))
                .unify(),
        )
        .and(with_db(db_pool.clone()))
        .and(with_facts(media_facts.clone()))
        .and_then(list_map_photos);

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
        .and(with_facts(media_facts.clone()))
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
        .and(with_facts(media_facts.clone()))
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
        .and(with_facts(media_facts.clone()))
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

    let api_photo_video_stream = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("video"))
        .and(warp::path("stream"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<StreamQuery>())
        .and(warp::header::headers_cloned())
        .and(with_db(db_pool.clone()))
        .and_then(stream_video);

    let api_photo_favorite = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path("favorite"))
        .and(warp::path::end())
        .and(warp::put())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<FavoriteRequest>())
        .and(with_db(db_pool.clone()))
        .and(with_facts(media_facts.clone()))
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
        .and(with_facts(media_facts.clone()))
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
        .and(with_facts(media_facts.clone()))
        .and_then(rotate_photo);

    let api_photo_delete = warp::path("api")
        .and(warp::path("photos"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and(with_cache(cache_manager.clone()))
        .and(with_facts(media_facts.clone()))
        .and_then(delete_photo);

    api_photos_list
        .or(api_photos_map)
        .or(api_photo_timeline)
        .or(api_photo_batch_delete)
        .or(api_photo_batch_favorite)
        .or(api_photo_batch_export)
        .or(api_photo_get)
        .or(api_photo_file)
        .or(api_photo_file_head)
        .or(api_photo_video)
        .or(api_photo_video_head)
        .or(api_photo_video_status)
        .or(api_photo_video_stream)
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
    use crate::media_facts::{MediaFacts, MediaFactsIndex};
    use crate::warp_helpers::handle_rejection;
    use chrono::{DateTime, TimeZone, Utc};
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

    /// The fingerprint the scanner stores for a file: byte length plus the
    /// modification time truncated to whole seconds (`src/file_scanner.rs`),
    /// which is exactly what `find_unchanged_photo` compares.
    fn scanner_identity(path: &Path) -> (i64, DateTime<Utc>) {
        let metadata = fs::metadata(path).expect("file metadata");
        let seconds = metadata
            .modified()
            .expect("mtime")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("mtime after the epoch")
            .as_secs();
        (
            metadata.len() as i64,
            DateTime::from_timestamp(seconds as i64, 0).expect("mtime in range"),
        )
    }

    /// Insert a row for a file that already exists, carrying the fingerprint
    /// the scanner would have stored for it, so the row starts out describing
    /// the file exactly.
    async fn create_row_for_file(db_pool: &DbPool, path: &Path, hash: &str, mime_type: &str) {
        let (file_size, date_modified) = scanner_identity(path);
        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: path.to_str().unwrap().to_string(),
            filename: path.file_name().unwrap().to_string_lossy().to_string(),
            file_size,
            mime_type: Some(mime_type.to_string()),
            taken_at: None,
            width: None,
            height: None,
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: json!({}),
            date_modified,
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        photo
            .create(db_pool)
            .await
            .expect("Failed to create test row");
    }

    /// Insert a row backed by a temp copy of a video fixture. Size and
    /// modification time are the copy's own (the scanner's fingerprint), so the
    /// row starts out describing the file exactly as the scanner would have
    /// stored it.
    async fn create_video_row(
        db_pool: &DbPool,
        temp_dir: &TempDir,
        hash: &str,
        fixture: &str,
        mime_type: &str,
    ) -> std::path::PathBuf {
        let filename = Path::new(fixture)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let temp_video = temp_dir.path().join(&filename);
        fs::copy(fixture, &temp_video).expect("Failed to copy test video");
        create_row_for_file(db_pool, &temp_video, hash, mime_type).await;
        temp_video
    }

    /// A minimal ISO-BMFF file whose `moov` holds no `mvhd`/`tkhd`/`mdhd` and
    /// no text date item: the container with no carrier an instant could go
    /// into, which `write_metadata` accepts (it has nothing to patch) and the
    /// readback then finds dateless.
    fn mp4_without_a_date_carrier() -> Vec<u8> {
        fn box_bytes(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut out = Vec::with_capacity(8 + body.len());
            out.extend_from_slice(&u32::try_from(8 + body.len()).unwrap().to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out
        }

        let mut bytes = box_bytes(b"ftyp", b"isom\x00\x00\x02\x00isomiso2mp41");
        let mut moov = box_bytes(b"free", &[0u8; 16]);
        moov.extend_from_slice(&box_bytes(b"udta", &box_bytes(b"free", &[0u8; 8])));
        bytes.extend_from_slice(&box_bytes(b"moov", &moov));
        bytes
    }

    /// Same fixture as `create_photo_row`, and additionally seeds the photo's
    /// file-derived capture date into `facts` — the DB stores no date. Returns
    /// `(hash, backing file path)`.
    async fn create_photo_row_at(
        db_pool: &DbPool,
        facts: &MediaFactsIndex,
        temp_dir: &TempDir,
        hash: &str,
        filename: &str,
        taken_at: &str,
    ) -> (String, PathBuf) {
        let test_image = Path::new("test-data/IMG_9377.jpg");
        let temp_image = temp_dir.path().join(filename);
        fs::copy(test_image, &temp_image).expect("Failed to copy test image");

        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: temp_image.to_str().unwrap().to_string(),
            filename: filename.to_string(),
            file_size: 12345,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
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
        facts.set(
            &photo.file_path,
            MediaFacts {
                taken_at: Some(
                    DateTime::parse_from_rfc3339(taken_at)
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                ..MediaFacts::default()
            },
        );

        (photo.hash_sha256, temp_image)
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
        build_test_routes_with_facts(db_pool, cache_dir, Arc::new(MediaFactsIndex::new()))
    }

    /// Same route set, but serving capture facts from `facts` — the tests that
    /// assert a date or coordinates must seed the index themselves.
    fn build_test_routes_with_facts(
        db_pool: DbPool,
        cache_dir: PathBuf,
        facts: Arc<MediaFactsIndex>,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        let data_path = cache_dir
            .parent()
            .map(|p| p.join("data"))
            .unwrap_or_else(|| cache_dir.join("data"));
        build_photo_routes(db_pool, facts, CacheManager::new(cache_dir), data_path)
            .recover(handle_rejection)
    }

    /// Collect a reply into the JSON value the client sees.
    async fn json_body(reply: impl warp::Reply) -> serde_json::Value {
        use std::future::poll_fn;
        use std::pin::Pin;
        use warp::hyper::body::Body as _;

        let response = warp::reply::Reply::into_response(reply);
        let mut body = response.into_body();
        let mut out = Vec::new();
        while let Some(Ok(frame)) = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
            if let Ok(data) = frame.into_data() {
                out.extend_from_slice(&data);
            }
        }
        serde_json::from_slice(&out).expect("JSON body")
    }

    #[tokio::test]
    async fn test_map_photos_returns_all_matches_beyond_page_limit() {
        let db_pool = create_in_memory_pool().await.expect("db");
        // Coordinates live in the index now (the file is their only home).
        let facts = Arc::new(crate::media_facts::MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );

        for index in 0..120 {
            let photo = crate::db::tests::create_test_photo(
                format!("map_{index}.jpg"),
                format!("map{index:03}"),
            );
            photo.create(&db_pool).await.unwrap();
            facts.set(
                &photo.file_path,
                crate::media_facts::MediaFacts {
                    taken_at: Some(Utc.with_ymd_and_hms(2020, 1, 1, 12, 0, 0).unwrap()),
                    latitude: Some(48.1),
                    longitude: Some(11.5),
                },
            );
        }

        let response = warp::test::request()
            .path("/api/photos/map")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["photos"].as_array().unwrap().len(), 120);
        // The map listing is not a page: coordinates travel with the payload.
        assert_eq!(body["photos"][0]["metadata"]["location"]["latitude"], 48.1);
    }

    #[tokio::test]
    async fn test_map_photos_applies_filters_and_sort() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let facts = Arc::new(crate::media_facts::test_facts_with_coords(&[
            ("./test/older.jpg", "2020-05-25T10:00:00Z", 52.5, 13.4),
            ("./test/newer.jpg", "2024-05-25T10:00:00Z", 52.5, 13.4),
        ]));
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );

        let mut older =
            crate::db::tests::create_test_photo("older.jpg".to_string(), "a".repeat(64));
        older.metadata = json!({ "location": { "city": "Berlin" } });
        older.create(&db_pool).await.unwrap();

        let mut newer =
            crate::db::tests::create_test_photo("newer.jpg".to_string(), "b".repeat(64));
        newer.metadata = json!({ "location": { "city": "Berlin" } });
        newer.create(&db_pool).await.unwrap();

        let response = warp::test::request()
            .path("/api/photos/map?q=location%3ABerlin&sort=date&order=asc")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        let photos = body["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[0]["filename"], "older.jpg");
        assert_eq!(photos[1]["filename"], "newer.jpg");
    }

    #[tokio::test]
    async fn test_map_photos_returns_no_coordinates_for_a_video() {
        let db_pool = create_in_memory_pool().await.expect("db");
        // Coordinates live in the index; a video's file yields a date but no
        // GPS, so the index entry has `taken_at` only.
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );

        let mut video = crate::db::tests::create_test_photo("clip.mp4".to_string(), "c".repeat(64));
        video.mime_type = Some("video/mp4".to_string());
        video.create(&db_pool).await.unwrap();
        let taken_at = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
        facts.set(
            &video.file_path,
            MediaFacts {
                taken_at: Some(taken_at),
                ..MediaFacts::default()
            },
        );

        // A second row that matches every OTHER dimension of the query — an
        // image (the default mime of a test photo) with a date AND coordinates
        // in the index — so the count assertion below is about the filter and
        // not about the library holding a single row: a dropped `type:video`
        // token would return this photo too.
        let still = crate::db::tests::create_test_photo("still.jpg".to_string(), "d".repeat(64));
        still.create(&db_pool).await.unwrap();
        facts.set(
            &still.file_path,
            MediaFacts {
                taken_at: Some(Utc.with_ymd_and_hms(2023, 6, 7, 8, 9, 10).unwrap()),
                latitude: Some(52.5),
                longitude: Some(13.4),
            },
        );

        let response = warp::test::request()
            .path("/api/photos/map?q=type:video")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        let photos = body["photos"].as_array().unwrap();
        assert_eq!(
            photos.len(),
            1,
            "type:video must select exactly the video, not the image beside it"
        );
        assert_eq!(photos[0]["filename"], "clip.mp4");
        assert_eq!(photos[0]["hash_sha256"], video.hash_sha256);

        // A video can never acquire map coordinates: no matter how the index
        // is seeded, the payload must carry no location pair...
        assert!(
            photos[0]["metadata"]["location"].get("latitude").is_none(),
            "a video must not expose metadata.location.latitude"
        );
        assert!(
            photos[0]["metadata"]["location"].get("longitude").is_none(),
            "a video must not expose metadata.location.longitude"
        );

        // ...while the date the file carries still travels with it.
        let serialized = photos[0]["taken_at"]
            .as_str()
            .expect("the video's file-derived date");
        assert_eq!(
            DateTime::parse_from_rfc3339(serialized)
                .unwrap()
                .with_timezone(&Utc),
            taken_at
        );
    }

    #[tokio::test]
    async fn test_map_photos_scopes_to_album() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let routes = build_test_routes(db_pool.clone(), PathBuf::from("/tmp/turbo-pix-test-cache"));

        let member =
            crate::db::tests::create_test_photo("member.jpg".to_string(), "member".to_string());
        member.create(&db_pool).await.unwrap();
        let stranger =
            crate::db::tests::create_test_photo("stranger.jpg".to_string(), "stranger".to_string());
        stranger.create(&db_pool).await.unwrap();

        let album = crate::albums::create_with_members(
            &db_pool,
            "Trip",
            std::slice::from_ref(&member.hash_sha256),
        )
        .await
        .unwrap();

        let response = warp::test::request()
            .path(&format!("/api/photos/map?album={}", album.id))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        let photos = body["photos"].as_array().unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0]["filename"], "member.jpg");
    }

    #[tokio::test]
    async fn test_map_photos_returns_empty_array_for_empty_library() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let routes = build_test_routes(db_pool, PathBuf::from("/tmp/turbo-pix-test-cache"));

        let response = warp::test::request()
            .path("/api/photos/map")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["photos"].as_array().unwrap().len(), 0);
    }

    /// The PATCH edits the file only: the response must report what the file
    /// now carries, at the precision the file can represent (FR-006), and the
    /// row's identity/bookkeeping must survive untouched.
    #[tokio::test]
    async fn test_update_photo_metadata_writes_the_file_and_reports_it() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let facts = Arc::new(MediaFactsIndex::new());
        let path = temp_image.to_string_lossy().to_string();

        let row_before = Photo::find_by_hash(&db_pool, &photo_hash)
            .await
            .unwrap()
            .unwrap();
        crate::albums::create_with_members(&db_pool, "Trip", std::slice::from_ref(&photo_hash))
            .await
            .expect("album membership");

        let result = update_photo_metadata(
            photo_hash.clone(),
            MetadataUpdateRequest {
                taken_at: Some("2024-03-15T14:30:00.123Z".to_string()),
                latitude: Some(40.7128),
                longitude: Some(-74.0060),
            },
            db_pool.clone(),
            facts.clone(),
        )
        .await;
        let body = json_body(result.expect("PATCH should succeed")).await;

        // The file is the only place the date and coordinates live now.
        let file_facts = crate::media_facts::read_media_facts(&temp_image);
        let file_taken_at = file_facts.taken_at.expect("date written to the file");
        assert_eq!(file_taken_at.to_rfc3339(), "2024-03-15T14:30:00+00:00");

        // FR-006: the reply carries the file's whole-second value, never the
        // sub-second precision the request asked for.
        assert_eq!(body["taken_at"], "2024-03-15T14:30:00Z");
        assert_ne!(body["taken_at"], "2024-03-15T14:30:00.123Z");
        let reported_lat = body["metadata"]["location"]["latitude"]
            .as_f64()
            .expect("latitude in the response");
        let reported_lon = body["metadata"]["location"]["longitude"]
            .as_f64()
            .expect("longitude in the response");
        assert!((reported_lat - file_facts.latitude.expect("file latitude")).abs() < 1e-4);
        assert!((reported_lon - file_facts.longitude.expect("file longitude")).abs() < 1e-4);
        assert_eq!(
            facts.get(&path).expect("index reloaded").taken_at,
            Some(file_taken_at)
        );

        // Identity and relationships are unchanged.
        let row_after = Photo::find_by_hash(&db_pool, &photo_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_after.hash_sha256, row_before.hash_sha256);
        assert_eq!(row_after.is_favorite, row_before.is_favorite);
        let members: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM album_members WHERE photo_hash = ?")
                .bind(&photo_hash)
                .fetch_one(&db_pool)
                .await
                .unwrap();
        assert_eq!(members, 1, "album membership must survive the edit");
    }

    /// A video's file cannot carry the edit; nothing (index entry, file) may
    /// change on the way to the error.
    #[tokio::test]
    async fn test_update_metadata_rejects_a_video_without_touching_anything() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let video_path = temp_dir.path().join("clip.mp4");
        fs::write(&video_path, b"not a video a writer could edit").expect("write video");

        let mut video = crate::db::tests::create_test_photo("clip.mp4".to_string(), "d".repeat(64));
        video.file_path = video_path.to_string_lossy().to_string();
        video.mime_type = Some("video/mp4".to_string());
        video.create(&db_pool).await.unwrap();

        let facts = Arc::new(MediaFactsIndex::new());
        let seeded = MediaFacts {
            taken_at: Some(Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()),
            ..MediaFacts::default()
        };
        facts.set(&video.file_path, seeded);
        let mtime_before = fs::metadata(&video_path).unwrap().modified().unwrap();

        let result = update_photo_metadata(
            video.hash_sha256.clone(),
            MetadataUpdateRequest {
                taken_at: Some("2024-03-15T14:30:00Z".to_string()),
                latitude: None,
                longitude: None,
            },
            db_pool.clone(),
            facts.clone(),
        )
        .await;

        assert!(result.is_err(), "a video's file cannot carry the edit");
        assert_eq!(facts.get(&video.file_path), Some(seeded));
        assert_eq!(
            fs::metadata(&video_path).unwrap().modified().unwrap(),
            mtime_before
        );
    }

    /// A failed write must not corrupt the index: the last good entry stays.
    #[tokio::test]
    async fn test_update_metadata_missing_file_keeps_the_index_unchanged() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let facts = Arc::new(MediaFactsIndex::new());
        let path = temp_image.to_string_lossy().to_string();
        let seeded = MediaFacts {
            taken_at: Some(Utc.with_ymd_and_hms(2020, 1, 1, 12, 0, 0).unwrap()),
            latitude: Some(1.0),
            longitude: Some(2.0),
        };
        facts.set(&path, seeded);
        fs::remove_file(&temp_image).expect("delete the backing file");

        let result = update_photo_metadata(
            photo_hash,
            MetadataUpdateRequest {
                taken_at: Some("2024-03-15T14:30:00Z".to_string()),
                latitude: None,
                longitude: None,
            },
            db_pool,
            facts.clone(),
        )
        .await;

        assert!(result.is_err(), "no file, no edit");
        assert_eq!(facts.get(&path), Some(seeded));
    }

    /// Two saves in a row leave the index (and every later read) at the file's
    /// last value — the file is the only source.
    #[tokio::test]
    async fn test_two_saves_leave_the_index_at_the_last_file_value() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let facts = Arc::new(MediaFactsIndex::new());
        let path = temp_image.to_string_lossy().to_string();

        for date in ["2024-03-15T14:30:00Z", "2025-07-01T09:15:30Z"] {
            let result = update_photo_metadata(
                photo_hash.clone(),
                MetadataUpdateRequest {
                    taken_at: Some(date.to_string()),
                    latitude: None,
                    longitude: None,
                },
                db_pool.clone(),
                facts.clone(),
            )
            .await;
            assert!(result.is_ok(), "save {date} should succeed");
        }

        let expected = DateTime::parse_from_rfc3339("2025-07-01T09:15:30Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(facts.get(&path).expect("indexed").taken_at, Some(expected));

        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );
        let response = warp::test::request()
            .path(&format!("/api/photos/{}", photo_hash))
            .reply(&routes)
            .await;
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["taken_at"], "2025-07-01T09:15:30Z");
    }

    /// The detail endpoint must serve the file's date and coordinates.
    #[tokio::test]
    async fn test_get_photo_returns_the_files_date_and_coordinates() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let facts = Arc::new(crate::media_facts::test_facts_with_coords(&[(
            "./test/single.jpg",
            "2024-05-01T10:00:00Z",
            48.1,
            11.5,
        )]));
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );

        let photo = crate::db::tests::create_test_photo("single.jpg".to_string(), "e".repeat(64));
        photo.create(&db_pool).await.unwrap();

        let response = warp::test::request()
            .path(&format!("/api/photos/{}", photo.hash_sha256))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["taken_at"], "2024-05-01T10:00:00Z");
        assert_eq!(body["metadata"]["location"]["latitude"], 48.1);
        assert_eq!(body["metadata"]["location"]["longitude"], 11.5);
    }

    /// Favoriting writes the flag but must still answer with the file's facts.
    #[tokio::test]
    async fn test_toggle_favorite_response_keeps_file_facts() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let facts = Arc::new(crate::media_facts::test_facts_with_coords(&[(
            "./test/fav.jpg",
            "2024-05-01T10:00:00Z",
            48.1,
            11.5,
        )]));
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );

        let photo = crate::db::tests::create_test_photo("fav.jpg".to_string(), "f".repeat(64));
        photo.create(&db_pool).await.unwrap();

        let response = warp::test::request()
            .method("PUT")
            .path(&format!("/api/photos/{}/favorite", photo.hash_sha256))
            .json(&serde_json::json!({ "is_favorite": true }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["is_favorite"], true);
        assert_eq!(body["taken_at"], "2024-05-01T10:00:00Z");
        assert_eq!(body["metadata"]["location"]["latitude"], 48.1);
        assert_eq!(body["metadata"]["location"]["longitude"], 11.5);
    }

    /// Rotating rewrites the pixels but the response still carries the file's
    /// capture facts (keyed by path, which survives the hash re-key).
    #[tokio::test]
    async fn test_rotate_response_keeps_file_facts() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().expect("temp dir");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let facts = Arc::new(MediaFactsIndex::new());
        facts.set(
            &temp_image.to_string_lossy(),
            MediaFacts {
                taken_at: Some(Utc.with_ymd_and_hms(2024, 5, 1, 10, 0, 0).unwrap()),
                latitude: Some(48.1),
                longitude: Some(11.5),
            },
        );
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );

        let response = warp::test::request()
            .method("POST")
            .path(&format!("/api/photos/{}/rotate", photo_hash))
            .json(&serde_json::json!({ "angle": 90 }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["taken_at"], "2024-05-01T10:00:00Z");
        assert_eq!(body["metadata"]["location"]["latitude"], 48.1);
        assert_eq!(body["metadata"]["location"]["longitude"], 11.5);
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
        let result = update_photo_metadata(
            photo_hash,
            update_req,
            db_pool,
            Arc::new(MediaFactsIndex::new()),
        )
        .await;

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
        let result = update_photo_metadata(
            photo_hash,
            update_req,
            db_pool,
            Arc::new(MediaFactsIndex::new()),
        )
        .await;

        // Verify the result is an error
        assert!(
            result.is_err(),
            "Handler should fail when GPS coordinates are not paired"
        );
    }

    #[tokio::test]
    async fn patch_metadata_edits_a_video_file_and_the_row() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().join("cache"),
            facts.clone(),
        );
        let hash = "1000000000000000000000000000000000000000000000000000000000000001";
        // The keys fixture has a location carrier, so the date AND the position
        // can both be written into the container itself.
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;
        let before = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(before.file_size, fs::metadata(&video).unwrap().len() as i64);

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({
                "taken_at": "2024-07-04T12:00:00Z",
                "latitude": 52.52,
                "longitude": 13.405,
            }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["taken_at"], "2024-07-04T12:00:00Z");
        assert_eq!(body["metadata"]["location"]["latitude"], 52.52);
        assert_eq!(body["metadata"]["location"]["longitude"], 13.405);
        // A `moov` rewrite of the same length never changes the file's size.
        assert_eq!(body["file_size"], before.file_size);

        // The container itself now carries the new instant and the position.
        let stored = crate::mp4_metadata::read_metadata(&video).unwrap();
        assert_eq!(
            stored.creation_time.unwrap().to_rfc3339(),
            "2024-07-04T12:00:00+00:00"
        );
        assert_eq!(
            stored.location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );

        // The row mirrors the file's scanner identity, while the date and the
        // coordinates it now carries are served from the facts index — the DB
        // stores neither.
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert!(row.taken_at.is_none());
        assert!(row.metadata["location"].get("latitude").is_none());
        assert!(row.metadata["location"].get("longitude").is_none());
        assert_eq!(row.file_size, before.file_size);
        let on_disk = fs::metadata(&video).unwrap().modified().unwrap();
        assert_eq!(
            row.date_modified.timestamp(),
            DateTime::<Utc>::from(on_disk).timestamp()
        );
        let entry = facts
            .get(&row.file_path)
            .expect("the save must publish the file's facts");
        assert_eq!(
            entry.taken_at.unwrap().to_rfc3339(),
            "2024-07-04T12:00:00+00:00"
        );
        assert_eq!(entry.latitude, Some(52.52));
        assert_eq!(entry.longitude, Some(13.405));
    }

    /// A save must not stamp the patched file's identity onto a row that
    /// describes a DIFFERENT file. When the video at this path was replaced
    /// since the last scan (a re-export, a copy from another tool), recording
    /// the new bytes here would erase the mismatch `find_unchanged_photo` keys
    /// on: every later scan would skip the file while the row kept the
    /// PREVIOUS file's facts (`metadata.video.*`, width/height/duration/
    /// orientation) forever. The stale fingerprint has to survive so the next
    /// scan re-extracts the file — the save's values live in the container,
    /// which is where that extraction re-derives them from.
    #[tokio::test]
    async fn a_video_save_does_not_stamp_a_file_replaced_behind_the_rows_back() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));

        // One row whose file was replaced by another video (size AND mtime
        // differ), and one whose file was re-copied (same bytes, a different
        // mtime — the scan compares both fields, so that mismatch counts too).
        let replaced_path = temp_dir.path().join("replaced.mp4");
        fs::copy("test-data/test_video_quicktime_keys.mp4", &replaced_path).unwrap();
        let replaced_hash = "1300000000000000000000000000000000000000000000000000000000000013";
        create_row_for_file(&db_pool, &replaced_path, replaced_hash, "video/mp4").await;
        let described = Photo::find_by_hash(&db_pool, replaced_hash)
            .await
            .unwrap()
            .unwrap();
        fs::copy("test-data/test_video_with_date.mp4", &replaced_path).unwrap();
        assert_ne!(
            fs::metadata(&replaced_path).unwrap().len() as i64,
            described.file_size
        );

        let recopied_path = temp_dir.path().join("recopied.mp4");
        fs::copy("test-data/test_video_quicktime_keys.mp4", &recopied_path).unwrap();
        let recopied_hash = "1400000000000000000000000000000000000000000000000000000000000014";
        create_row_for_file(&db_pool, &recopied_path, recopied_hash, "video/mp4").await;
        let recopied_described = Photo::find_by_hash(&db_pool, recopied_hash)
            .await
            .unwrap()
            .unwrap();
        // Move the file's mtime a minute back, as a re-copy out of an archive
        // would: the bytes are the same, the row's timestamp no longer is.
        let shifted = fs::metadata(&recopied_path).unwrap().modified().unwrap()
            - std::time::Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(&recopied_path)
            .unwrap()
            .set_modified(shifted)
            .unwrap();
        assert_ne!(
            scanner_identity(&recopied_path).1,
            recopied_described.date_modified
        );

        for hash in [replaced_hash, recopied_hash] {
            let response = warp::test::request()
                .method("PATCH")
                .path(&format!("/api/photos/{}/metadata", hash))
                .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
                .reply(&routes)
                .await;
            assert_eq!(response.status(), 200, "{hash}");
        }

        // The containers really did take the date...
        for path in [&replaced_path, &recopied_path] {
            assert_eq!(
                crate::mp4_metadata::read_metadata(path)
                    .unwrap()
                    .creation_time
                    .unwrap()
                    .to_rfc3339(),
                "2024-07-04T12:00:00+00:00"
            );
        }

        // ...but neither row was handed the new bytes' identity, so both files
        // still read as changed and the next scan re-extracts them.
        let row = Photo::find_by_hash(&db_pool, replaced_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.file_size, described.file_size);
        assert_eq!(row.date_modified, described.date_modified);
        let row = Photo::find_by_hash(&db_pool, recopied_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.file_size, recopied_described.file_size);
        assert_eq!(row.date_modified, recopied_described.date_modified);

        for path in [&replaced_path, &recopied_path] {
            let (size, mtime) = scanner_identity(path);
            assert!(
                Photo::find_unchanged_photo(&db_pool, path.to_str().unwrap(), size, mtime)
                    .await
                    .unwrap()
                    .is_none(),
                "{} must still read as changed after the save",
                path.display()
            );
        }
    }

    /// The other half of that rule: an ordinary save — the row already
    /// describes the file at that path — records the patched file's identity,
    /// which is what keeps `find_unchanged_photo` matching (no re-extraction,
    /// no thumbnail regeneration, no transcode cache invalidation) afterwards.
    #[tokio::test]
    async fn a_video_save_on_a_row_that_describes_the_file_records_its_identity() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "1500000000000000000000000000000000000000000000000000000000000015";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes)
            .await;
        assert_eq!(response.status(), 200);

        let (size, mtime) = scanner_identity(&video);
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(row.file_size, size);
        assert_eq!(row.date_modified, mtime);
        assert!(
            Photo::find_unchanged_photo(&db_pool, row.file_path.as_str(), size, mtime)
                .await
                .unwrap()
                .is_some(),
            "an ordinary save must leave the row matching the file"
        );
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["file_size"], size);
    }

    /// A date save on a container whose `moov` holds no date carrier must not
    /// report the requested instant: nothing date-shaped was written, so the
    /// row and the response would otherwise claim a date the file does not
    /// have. The save is refused with an existing code and the file rolled back
    /// to where it was, leaving it and the row in agreement.
    #[tokio::test]
    async fn a_date_save_on_a_container_without_a_date_carrier_is_refused() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "1600000000000000000000000000000000000000000000000000000000000016";
        let video = temp_dir.path().join("no_date_carrier.mp4");
        fs::write(&video, mp4_without_a_date_carrier()).unwrap();
        create_row_for_file(&db_pool, &video, hash, "video/mp4").await;
        let before_bytes = fs::read(&video).unwrap();
        let before_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 422);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error_code"], "unrepresentable_value");

        // The refusal left the file and the row exactly as they were.
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
        let after_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(after_row.taken_at, before_row.taken_at);
        assert_eq!(after_row.metadata, before_row.metadata);
        assert_eq!(after_row.file_size, before_row.file_size);
        assert_eq!(after_row.date_modified, before_row.date_modified);
        assert_eq!(after_row.updated_at, before_row.updated_at);
    }

    /// `mp4_metadata::restore` refuses unless the file still carries the exact
    /// modification time the write recorded, and that equality rests on the
    /// writer's own `set_modified`, which only WARNS when the kernel refuses it.
    /// A save whose clock was not wound back would leave the container patched
    /// while the undo refuses to run — the file and the row permanently
    /// disagreeing, and no later scan repairs it, because the row's fingerprint
    /// still matches the patched file. So the rollback puts the pre-write
    /// instant back and retries: the retry succeeding IS the proof that the
    /// token recorded that instant, since `restore` accepts nothing else.
    #[tokio::test]
    async fn a_rollback_re_arms_the_modification_time_the_write_recorded() {
        let temp_dir = TempDir::new().unwrap();
        let video = temp_dir.path().join("rearm_clock.mp4");
        fs::copy("test-data/test_video_with_date.mp4", &video).expect("fixture");
        let before_bytes = fs::read(&video).unwrap();
        let before_modified = fs::metadata(&video).unwrap().modified().unwrap();

        // GIVEN: a real write, so the token holds this file's real pre-write
        // clock, and a file whose clock the kernel did not wind back
        let edit = crate::mp4_metadata::VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse::<DateTime<Utc>>().unwrap()),
            latitude: None,
            longitude: None,
        };
        let write = crate::mp4_metadata::write_metadata(&video, &edit).expect("write");
        assert_ne!(
            fs::read(&video).unwrap(),
            before_bytes,
            "the fixture must carry a date this write replaces, or nothing was patched"
        );
        set_modified(&video, before_modified + std::time::Duration::from_secs(7))
            .expect("the kernel refused the clock the writer needs");

        // THEN: the plain restore refuses — the token's guard is real
        assert!(
            crate::mp4_metadata::restore(&write.undo).is_err(),
            "restore must refuse a file whose clock moved, or it would graft an \
             old `moov` onto whatever is there now"
        );

        // AND: the rollback re-arms the recorded clock and undoes the write
        roll_back_container(&video, &write, Some(before_modified)).expect("rollback");
        assert_eq!(
            fs::read(&video).unwrap(),
            before_bytes,
            "the container must be byte-identical to where it started"
        );
        assert_eq!(
            fs::metadata(&video).unwrap().modified().unwrap(),
            before_modified,
            "the file's own identity must be what the row still describes"
        );
    }

    /// The container keeps the carrier's own representation, and the row (and
    /// this response) must describe the file, not the request: a position is
    /// re-rendered in the shape the carrier already had (four decimals here, so
    /// it rounds — about 11 m at that width) and an instant is whole seconds in
    /// the binary boxes. `metadata_extractor` re-derives both from those
    /// carriers, so a row holding the request would silently move on the next
    /// scan of a changed file.
    #[tokio::test]
    async fn patch_metadata_mirrors_the_values_the_carrier_actually_stored() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().join("cache"),
            facts.clone(),
        );
        let hash = "1100000000000000000000000000000000000000000000000000000000000011";
        // The keys fixture's location carrier keeps four decimals.
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;

        let requested_latitude = 52.123456789;
        let requested_longitude = 13.405678901;
        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({
                "taken_at": "2024-07-04T12:00:00.840000Z",
                "latitude": requested_latitude,
                "longitude": requested_longitude,
            }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);

        // What the container holds is the answer, not what was asked for.
        let stored = crate::mp4_metadata::read_metadata(&video).unwrap();
        let (applied_latitude, applied_longitude) =
            crate::mp4_metadata::parse_iso6709(stored.location_iso6709.as_deref().unwrap())
                .expect("the written carrier must parse");
        assert_ne!(
            applied_latitude, requested_latitude,
            "the fixture's carrier must round the request, or this test proves nothing"
        );
        let applied_taken_at = stored.creation_time.expect("the write set `mvhd`");
        let requested_taken_at = "2024-07-04T12:00:00.840000Z"
            .parse::<DateTime<Utc>>()
            .unwrap();
        assert_ne!(
            applied_taken_at, requested_taken_at,
            "the sub-second the container cannot hold must not be mirrored"
        );

        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["metadata"]["location"]["latitude"], applied_latitude);
        assert_eq!(body["metadata"]["location"]["longitude"], applied_longitude);
        assert_eq!(
            body["taken_at"],
            serde_json::to_value(applied_taken_at).unwrap()
        );

        // AND: the row stores neither the date nor the coordinates — both are
        // file facts, and the index is what a later read serves them from.
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert!(row.taken_at.is_none());
        assert!(row.metadata["location"].get("latitude").is_none());
        assert!(row.metadata["location"].get("longitude").is_none());
        let entry = facts
            .get(&row.file_path)
            .expect("the save must publish the file's facts");
        assert_eq!(entry.taken_at, Some(applied_taken_at));
        assert_eq!(entry.latitude, Some(applied_latitude));
        assert_eq!(entry.longitude, Some(applied_longitude));
    }

    /// The place name in a row was geocoded from the coordinates the file
    /// held. A save that moves the photo replaces them, and nothing else would
    /// ever correct the name: the save restates the file's fingerprint, so
    /// nothing downstream of the scan would, and the resolver skips the row for
    /// as long as its flag says it is resolved. So the save drops the name it
    /// invalidates, re-queues the row, and reports the same state — a new pin
    /// next to the old city's name would be a lie.
    #[tokio::test]
    async fn patch_metadata_drops_the_resolved_name_when_the_save_moves_the_video() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().join("cache"),
            facts.clone(),
        );
        let hash = "1300000000000000000000000000000000000000000000000000000000000013";
        create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;
        let file_path = Photo::find_by_hash(&db_pool, hash)
            .await
            .unwrap()
            .unwrap()
            .file_path;

        // GIVEN: a located video the resolver named
        let located = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 48.2082, "longitude": 16.3737 }))
            .reply(&routes)
            .await;
        assert_eq!(located.status(), 200);
        crate::db::update_photo_city(&db_pool, &file_path, Some("Vienna"))
            .await
            .expect("resolve");

        // WHEN: a save moves it to a new position
        let moved = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 52.52, "longitude": 13.405 }))
            .reply(&routes)
            .await;

        // THEN: the name is gone from the response and from the row, and the
        // row is queued for the resolver again with the NEW position
        assert_eq!(moved.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(moved.body()).unwrap();
        assert_eq!(body["metadata"]["location"]["latitude"], 52.52);
        assert!(
            body["metadata"]["location"]["city"].is_null(),
            "the response must not pair the new position with the old name: {}",
            body["metadata"]
        );
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        // The row stores neither the date nor the coordinates: they are file
        // facts, and the response above already carries them.
        assert!(row.taken_at.is_none());
        assert!(row.metadata["location"].get("latitude").is_none());
        assert!(row.metadata["location"].get("longitude").is_none());
        assert!(
            row.metadata["location"]["city"].is_null(),
            "a name geocoded from the position this save replaced must not survive it: {}",
            row.metadata
        );
        assert_eq!(
            crate::db::get_photos_needing_geo_resolution(&db_pool, &facts)
                .await
                .expect("candidates"),
            vec![(file_path.clone(), 52.52, 13.405)],
            "nothing else would ever queue this row again"
        );
    }

    /// The counterpart: a save that does not move the photo leaves its resolved
    /// name alone. The date-only save the editor sends for an untouched
    /// position is the case that matters — the coordinates are the very ones
    /// the name was geocoded for, and re-queuing the row would only cost a
    /// reverse-geocode request for a name the row already carries.
    #[tokio::test]
    async fn patch_metadata_keeps_the_resolved_name_when_the_save_holds_the_position() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().join("cache"),
            facts.clone(),
        );
        let hash = "1400000000000000000000000000000000000000000000000000000000000014";
        create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;
        let file_path = Photo::find_by_hash(&db_pool, hash)
            .await
            .unwrap()
            .unwrap()
            .file_path;

        // GIVEN: a located video the resolver named
        let located = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 48.2082, "longitude": 16.3737 }))
            .reply(&routes)
            .await;
        assert_eq!(located.status(), 200);
        crate::db::update_photo_city(&db_pool, &file_path, Some("Vienna"))
            .await
            .expect("resolve");

        // WHEN: a save changes nothing but the date
        let dated = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes)
            .await;

        // THEN: the name is still there, in the response and in the row, and
        // the row stays out of the resolver's queue
        assert_eq!(dated.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(dated.body()).unwrap();
        assert_eq!(body["metadata"]["location"]["city"], "Vienna");
        assert_eq!(body["metadata"]["location"]["latitude"], 48.2082);
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(row.metadata["location"]["city"], "Vienna");
        // The row stores no coordinates: they are file facts.
        assert!(row.metadata["location"].get("latitude").is_none());
        assert!(
            crate::db::get_photos_needing_geo_resolution(&db_pool, &facts)
                .await
                .expect("candidates")
                .is_empty(),
            "a save that held the position must not invalidate its name: {}",
            row.metadata
        );
    }

    /// The container, not the request, decides whether the pin moved. This
    /// fixture's location carrier keeps four decimals, so a request carrying
    /// more precision than that rounds back to the position the row already
    /// holds. Deciding from the request would report a move that never
    /// happened: the name of that position is still correct, and dropping it
    /// costs a reverse-geocode request that resolves to the same string.
    #[tokio::test]
    async fn patch_metadata_keeps_the_resolved_name_when_the_request_rounds_back_to_the_stored_position(
    ) {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().join("cache"),
            facts.clone(),
        );
        let hash = "1500000000000000000000000000000000000000000000000000000000000015";
        create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;
        let file_path = Photo::find_by_hash(&db_pool, hash)
            .await
            .unwrap()
            .unwrap()
            .file_path;

        // GIVEN: a located video the resolver named
        let located = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 48.2082, "longitude": 16.3737 }))
            .reply(&routes)
            .await;
        assert_eq!(located.status(), 200);
        crate::db::update_photo_city(&db_pool, &file_path, Some("Vienna"))
            .await
            .expect("resolve");

        // WHEN: a save asks for that very pin with more precision than the
        // carrier can hold, so what lands in the file is the stored pin
        let rounded = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({
                "latitude": 48.2082 + 0.000000004,
                "longitude": 16.3737 + 0.000000004,
            }))
            .reply(&routes)
            .await;

        // THEN: the applied position is the stored one, so nothing moved
        assert_eq!(rounded.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(rounded.body()).unwrap();
        assert_eq!(body["metadata"]["location"]["latitude"], 48.2082);
        assert_eq!(
            body["metadata"]["location"]["city"], "Vienna",
            "a request that rounds back to the stored pin moved nothing: {}",
            body["metadata"]
        );
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(row.metadata["location"]["city"], "Vienna");
        // The row stores no coordinates: they are file facts, and the response
        // above already carries the applied pin.
        assert!(row.metadata["location"].get("latitude").is_none());
        assert!(
            crate::db::get_photos_needing_geo_resolution(&db_pool, &facts)
                .await
                .expect("candidates")
                .is_empty(),
            "the pin did not move, so the name must not be re-queued: {}",
            row.metadata
        );
    }

    /// A client that diffs the PATCH answer against a refetch — or hands the
    /// answer straight to a state store and reconciles later — must not see the
    /// dropped place name as a JSON `null` in one representation and as an
    /// absent member in the other: both sides drop the member. The response
    /// additionally carries the file's date and coordinates from the facts
    /// index, which the row never stores.
    #[tokio::test]
    async fn patch_metadata_answers_with_the_dropped_name_absent_on_both_sides() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "1600000000000000000000000000000000000000000000000000000000000016";
        create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;
        let file_path = Photo::find_by_hash(&db_pool, hash)
            .await
            .unwrap()
            .unwrap()
            .file_path;

        // GIVEN: a located video the resolver named
        let located = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 48.2082, "longitude": 16.3737 }))
            .reply(&routes)
            .await;
        assert_eq!(located.status(), 200);
        crate::db::update_photo_city(&db_pool, &file_path, Some("Vienna"))
            .await
            .expect("resolve");

        // WHEN: a save moves it, which is the path that drops the name
        let moved = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 52.52, "longitude": 13.405 }))
            .reply(&routes)
            .await;

        // THEN: the response serves the file's facts and the row stores none
        // of them
        assert_eq!(moved.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(moved.body()).unwrap();
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(body["metadata"]["location"]["latitude"], 52.52);
        assert_eq!(body["metadata"]["location"]["longitude"], 13.405);
        assert!(row.metadata["location"].get("latitude").is_none());
        assert!(row.metadata["location"].get("longitude").is_none());
        // The name is ABSENT on both sides, not a JSON null the client has to
        // filter: RFC 7396's explicit null removes the member from the row, and
        // the response mirrors the row's document for it. A future "keep the
        // key, set it null" on either side alone would show up here as one of
        // them carrying `"city": null`.
        for (side, location) in [
            ("response", &body["metadata"]["location"]),
            ("row", &row.metadata["location"]),
        ] {
            assert!(
                location.get("city").is_none(),
                "the {side} must carry no `city` member at all: {location}"
            );
        }
    }

    /// A row deleted between `find_by_hash` and the mirror (another window
    /// deleting the same video) must not answer 200 with values no row holds:
    /// the container is rewritten by then, so the update has to fail and let
    /// the rollback put the file back.
    #[tokio::test]
    async fn a_row_deleted_during_the_save_fails_and_rolls_the_file_back() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "1200000000000000000000000000000000000000000000000000000000000012";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;
        let before_bytes = fs::read(&video).unwrap();
        let before_mtime = fs::metadata(&video).unwrap().modified().unwrap();
        let photo = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();

        // The handler re-reads the row under its own lock, so a row that is
        // already gone takes `find_by_hash`'s 404 path. The hazard is the row
        // that vanishes between that read and the write: a `BEFORE UPDATE`
        // trigger that deletes it makes exactly that happen — the container is
        // already rewritten, and the mirror then matches no row.
        sqlx::query(
            "CREATE TRIGGER delete_row_at_write BEFORE UPDATE ON photos \
             BEGIN DELETE FROM photos WHERE hash_sha256 = OLD.hash_sha256; END",
        )
        .execute(&db_pool)
        .await
        .expect("trigger");

        let facts = MediaFactsIndex::new();
        let rejection = match apply_video_metadata_edit(
            photo,
            crate::mp4_metadata::VideoMetadataEdit {
                taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
                ..Default::default()
            },
            &db_pool,
            &facts,
        )
        .await
        {
            Ok(_) => panic!("a row that no longer exists must not answer 200"),
            Err(rejection) => rejection,
        };
        assert!(
            rejection.find::<DatabaseError>().is_some(),
            "the mirror failure must take the generic 500 path, not a coded refusal"
        );

        // AND: the file is exactly as the scanner last saw it
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
        assert_eq!(
            fs::metadata(&video).unwrap().modified().unwrap(),
            before_mtime
        );
    }

    #[tokio::test]
    async fn patch_metadata_refuses_a_video_without_a_location_carrier() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "2000000000000000000000000000000000000000000000000000000000000002";
        // This fixture carries a date but no location carrier at all.
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_with_date.mp4",
            "video/mp4",
        )
        .await;
        let before_bytes = fs::read(&video).unwrap();
        let before_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 52.52, "longitude": 13.405 }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 422);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error_code"], "no_location_carrier");

        // Refused before a byte was written and before the row was touched.
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
        let after_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(after_row.taken_at, before_row.taken_at);
        assert_eq!(after_row.metadata, before_row.metadata);
        assert_eq!(after_row.date_modified, before_row.date_modified);
        assert_eq!(after_row.updated_at, before_row.updated_at);
    }

    #[tokio::test]
    async fn patch_metadata_refuses_a_matroska_video_with_a_machine_readable_code() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "3000000000000000000000000000000000000000000000000000000000000003";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_long.mkv",
            "video/x-matroska",
        )
        .await;
        let before_bytes = fs::read(&video).unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes)
            .await;

        // A container this project cannot rewrite is a 415 with a code the
        // client can translate — never the generic 500.
        assert_eq!(response.status(), 415);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error_code"], "unsupported_container");
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
    }

    #[tokio::test]
    async fn an_empty_video_request_touches_nothing() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "4000000000000000000000000000000000000000000000000000000000000004";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_with_date.mp4",
            "video/mp4",
        )
        .await;
        let before_bytes = fs::read(&video).unwrap();
        let before_mtime = fs::metadata(&video).unwrap().modified().unwrap();
        let before_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({}))
            .reply(&routes)
            .await;

        // FR-013: an empty request answers the row it found and rewrites
        // neither the container nor the row.
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body, serde_json::to_value(&before_row).unwrap());
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
        assert_eq!(
            fs::metadata(&video).unwrap().modified().unwrap(),
            before_mtime
        );
        let after_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&after_row).unwrap(),
            serde_json::to_value(&before_row).unwrap()
        );
    }

    #[tokio::test]
    async fn a_failed_row_write_rolls_the_file_back() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "5000000000000000000000000000000000000000000000000000000000000005";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;
        let before_bytes = fs::read(&video).unwrap();
        let before_mtime = fs::metadata(&video).unwrap().modified().unwrap();
        let before_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();

        // Fail the row write only: the SELECTs still work, so the handler gets
        // as far as the mirror and has to undo the file write.
        sqlx::query(
            "CREATE TRIGGER refuse_photo_update BEFORE UPDATE ON photos \
             BEGIN SELECT RAISE(ABORT, 'row write refused'); END",
        )
        .execute(&db_pool)
        .await
        .expect("trigger");

        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 500);
        // The file is byte-identical and its mtime is the one the scanner saw,
        // so the next scan still matches this row as unchanged.
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
        assert_eq!(
            fs::metadata(&video).unwrap().modified().unwrap(),
            before_mtime
        );
        let after_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(after_row.taken_at, before_row.taken_at);
        assert_eq!(after_row.date_modified, before_row.date_modified);
    }

    // Multi-threaded on purpose: the two saves must be kept apart by the
    // handler's lock, not by a single-threaded runtime that can only switch
    // between them at an `await`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_saves_do_not_interleave() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().join("cache"),
            facts.clone(),
        );
        let hash = "6000000000000000000000000000000000000000000000000000000000000006";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_quicktime_keys.mp4",
            "video/mp4",
        )
        .await;

        let date_req = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes);
        let location_req = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "latitude": 52.52, "longitude": 13.405 }))
            .reply(&routes);

        let (date_response, location_response) = tokio::join!(date_req, location_req);
        assert_eq!(date_response.status(), 200);
        assert_eq!(location_response.status(), 200);

        // Neither save's write is lost, and the container still parses.
        let stored = crate::mp4_metadata::read_metadata(&video).unwrap();
        assert_eq!(
            stored.creation_time.unwrap().to_rfc3339(),
            "2024-07-04T12:00:00+00:00"
        );
        assert_eq!(
            stored.location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );

        // The facts index carries both saves too: the second handler's reload
        // sees the file the first one committed. The row stores neither the
        // date nor the coordinates.
        let row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert!(row.taken_at.is_none());
        assert!(row.metadata["location"].get("latitude").is_none());
        assert!(row.metadata["location"].get("longitude").is_none());
        let entry = facts
            .get(&row.file_path)
            .expect("a save must publish the file's facts");
        assert_eq!(
            entry.taken_at.unwrap().to_rfc3339(),
            "2024-07-04T12:00:00+00:00"
        );
        assert_eq!(entry.latitude, Some(52.52));
        assert_eq!(entry.longitude, Some(13.405));
    }

    #[tokio::test]
    async fn video_refusals_carry_distinct_machine_readable_codes() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "7000000000000000000000000000000000000000000000000000000000000007";
        create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_with_date.mp4",
            "video/mp4",
        )
        .await;

        // Values the container itself refuses: the photo path answers an
        // out-of-range coordinate with a bare validation error, the container
        // write names what was wrong.
        for (payload, expected_code) in [
            (
                json!({ "latitude": 91.0, "longitude": 0.0 }),
                "invalid_coordinates",
            ),
            // The writer's representable range ends in 2040.
            (
                json!({ "taken_at": "2050-01-01T00:00:00Z" }),
                "invalid_date",
            ),
        ] {
            let response = warp::test::request()
                .method("PATCH")
                .path(&format!("/api/photos/{}/metadata", hash))
                .json(&payload)
                .reply(&routes)
                .await;

            assert_eq!(response.status(), 400, "{}", expected_code);
            let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
            assert_eq!(body["error_code"], expected_code);
        }
    }

    /// FR-010 names an unparsable date as a refusal class of its own, so a
    /// video row answers with the code the frontend localizes; FR-011 keeps the
    /// photo path's bare validation error exactly as it was.
    #[tokio::test]
    async fn an_unparsable_date_is_coded_on_the_video_path_only() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));

        let video_hash = "a00000000000000000000000000000000000000000000000000000000000000a";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            video_hash,
            "test-data/test_video_with_date.mp4",
            "video/mp4",
        )
        .await;
        let before_bytes = fs::read(&video).unwrap();
        let before_row = Photo::find_by_hash(&db_pool, video_hash)
            .await
            .unwrap()
            .unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", video_hash))
            .json(&json!({ "taken_at": "not-a-date" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error_code"], "invalid_date");

        // Refused before the container or the row was touched.
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
        let after_row = Photo::find_by_hash(&db_pool, video_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_row.taken_at, before_row.taken_at);
        assert_eq!(after_row.metadata, before_row.metadata);
        assert_eq!(after_row.updated_at, before_row.updated_at);

        let (photo_hash, _temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", photo_hash))
            .json(&json!({ "taken_at": "not-a-date" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert!(
            body.get("error_code").is_none(),
            "unexpected error_code: {body}"
        );
    }

    /// Pins the whole refusal table. Three rows of it (fragmented/no-room,
    /// unrepresentable, I/O) have no fixture that reaches them through the
    /// route, so the classification is asserted directly.
    #[test]
    fn video_refusal_table_maps_every_container_failure() {
        use crate::mp4_metadata::Mp4MetadataError;

        for (err, expected_status, expected_code) in [
            (
                Mp4MetadataError::UnsupportedContainer("matroska".to_string()),
                415,
                "unsupported_container",
            ),
            (Mp4MetadataError::Fragmented, 422, "no_writable_slot"),
            (Mp4MetadataError::NoRoom("moov"), 422, "no_writable_slot"),
            (
                Mp4MetadataError::NoLocationCarrier,
                422,
                "no_location_carrier",
            ),
            (
                Mp4MetadataError::Unrepresentable("\u{a9}day"),
                422,
                "unrepresentable_value",
            ),
            (Mp4MetadataError::InvalidDate, 400, "invalid_date"),
            (
                Mp4MetadataError::InvalidCoordinates,
                400,
                "invalid_coordinates",
            ),
            (Mp4MetadataError::MissingFile, 404, "file_missing"),
            (
                Mp4MetadataError::ReadOnly("read-only mount".to_string()),
                403,
                "file_read_only",
            ),
        ] {
            let rejection = video_metadata_rejection(err);
            let refusal = rejection
                .find::<VideoMetadataError>()
                .unwrap_or_else(|| panic!("{expected_code}: expected a coded refusal"));
            assert_eq!(refusal.status.as_u16(), expected_status, "{expected_code}");
            assert_eq!(refusal.code, expected_code);
        }

        // An I/O failure is a server fault, not something the client can act
        // on: it takes the generic 500 with no code at all.
        let rejection =
            video_metadata_rejection(Mp4MetadataError::Io(std::io::Error::other("disk gone")));
        assert!(rejection.find::<VideoMetadataError>().is_none());
        assert!(rejection.find::<DatabaseError>().is_some());
    }

    #[tokio::test]
    async fn patch_metadata_reports_a_video_file_that_is_gone() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "8000000000000000000000000000000000000000000000000000000000000008";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_with_date.mp4",
            "video/mp4",
        )
        .await;
        fs::remove_file(&video).unwrap();
        let before_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 404);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error_code"], "file_missing");
        let after_row = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        assert_eq!(after_row.taken_at, before_row.taken_at);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn patch_metadata_refuses_a_read_only_video() {
        use std::os::unix::fs::PermissionsExt;

        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let routes = build_test_routes(db_pool.clone(), temp_dir.path().join("cache"));
        let hash = "9000000000000000000000000000000000000000000000000000000000000009";
        let video = create_video_row(
            &db_pool,
            &temp_dir,
            hash,
            "test-data/test_video_with_date.mp4",
            "video/mp4",
        )
        .await;
        let before_bytes = fs::read(&video).unwrap();
        fs::set_permissions(&video, fs::Permissions::from_mode(0o444)).unwrap();

        // Root may write a 0444 file regardless, so the permission bit has to
        // be the deciding factor for this test to mean anything.
        if fs::OpenOptions::new().write(true).open(&video).is_ok() {
            eprintln!("Skipping read-only test: this user may write a 0444 file");
            return;
        }

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/photos/{}/metadata", hash))
            .json(&json!({ "taken_at": "2024-07-04T12:00:00Z" }))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 403);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error_code"], "file_read_only");
        assert_eq!(fs::read(&video).unwrap(), before_bytes);
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
    async fn test_list_photos_month_range_filter() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().to_path_buf(),
            facts.clone(),
        );

        for (hash, filename, taken_at) in [
            ("a", "mar2012.jpg", "2012-03-15T10:00:00Z"),
            ("b", "aug2015.jpg", "2015-08-31T23:30:00Z"),
            ("c", "sep2015.jpg", "2015-09-01T00:00:00Z"),
        ] {
            create_photo_row_at(
                &db_pool,
                &facts,
                &temp_dir,
                &hash.repeat(64),
                filename,
                taken_at,
            )
            .await;
        }

        let response = warp::test::request()
            .path("/api/photos?year=2012&month=3&to_year=2015&to_month=8")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["total"], 2, "range must be inclusive on both bounds");
        let filenames: Vec<&str> = body["photos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["filename"].as_str().unwrap())
            .collect();
        assert!(filenames.contains(&"mar2012.jpg"));
        assert!(filenames.contains(&"aug2015.jpg"));
        assert!(!filenames.contains(&"sep2015.jpg"));
    }

    #[tokio::test]
    async fn test_list_photos_sql_sort_orders_and_still_carries_facts() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            PathBuf::from("/tmp/turbo-pix-test-cache"),
            facts.clone(),
        );

        // `sort=name` is a SQL fast-path column sort, but the payload contract
        // still holds: every row must be enriched from the index.
        for (hash, filename, taken_at) in [
            ("a", "alpha.jpg", "2015-06-20T10:00:00Z"),
            ("b", "bravo.jpg", "2024-01-02T03:04:05Z"),
        ] {
            let photo = crate::db::tests::create_test_photo(filename.to_string(), hash.repeat(64));
            photo.create(&db_pool).await.unwrap();
            facts.set(
                &photo.file_path,
                MediaFacts {
                    taken_at: Some(
                        DateTime::parse_from_rfc3339(taken_at)
                            .unwrap()
                            .with_timezone(&Utc),
                    ),
                    latitude: Some(48.1372),
                    longitude: Some(11.5755),
                },
            );
        }

        let response = warp::test::request()
            .path("/api/photos?sort=name&order=asc")
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        let photos = body["photos"].as_array().unwrap();
        let filenames: Vec<&str> = photos
            .iter()
            .map(|p| p["filename"].as_str().unwrap())
            .collect();
        assert_eq!(filenames, ["alpha.jpg", "bravo.jpg"]);

        for (row, expected_date) in photos
            .iter()
            .zip(["2015-06-20T10:00:00Z", "2024-01-02T03:04:05Z"])
        {
            let serialized = row["taken_at"]
                .as_str()
                .expect("the fast path must enrich taken_at");
            assert_eq!(
                DateTime::parse_from_rfc3339(serialized)
                    .unwrap()
                    .with_timezone(&Utc),
                DateTime::parse_from_rfc3339(expected_date)
                    .unwrap()
                    .with_timezone(&Utc)
            );
            assert_eq!(row["metadata"]["location"]["latitude"], 48.1372);
            assert_eq!(row["metadata"]["location"]["longitude"], 11.5755);
        }
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
    async fn test_video_stream_route_reaches_handler_from_an_empty_source() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let (photo_hash, temp_image) = setup_test_photo(&db_pool, &temp_dir).await;
        fs::write(&temp_image, b"").expect("Failed to truncate test image");
        let routes = build_test_routes(db_pool, temp_dir.path().join("cache"));

        let response = warp::test::request()
            .method("GET")
            .path(&format!(
                "/api/photos/{}/video/stream?mode=remux&start=0",
                photo_hash
            ))
            .reply(&routes)
            .await;

        // Unregistered, this path falls through to the 404 rejection; only the
        // stream handler answers an empty source with this warning.
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["x-transcode-warning"], "empty");
        assert_eq!(response.headers()["content-length"], "0");
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
        // The machine-readable code is emitted only where the client is meant
        // to act on it; the photo path's error body is otherwise unchanged.
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert!(
            body.get("error_code").is_none(),
            "unexpected error_code: {body}"
        );
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
    async fn test_map_invalid_query_param_returns_bad_request() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let routes = build_test_routes(db_pool, PathBuf::from("/tmp/turbo-pix-test-cache"));

        // `/api/photos/map` shares its path shape with the `{hash}` route, whose
        // NotFoundError used to win the combined rejection and answer 404
        // "Photo not found". The body message is what discriminates the two
        // (both branches are 4xx).
        for path in [
            "/api/photos/map?year=abc",
            "/api/photos/map?album=99999999999999999999",
        ] {
            let response = warp::test::request().path(path).reply(&routes).await;

            assert_eq!(response.status(), 400, "{} must be a client error", path);
            let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
            assert_eq!(body["error"], "Invalid query parameters", "{}", path);
        }

        // The `{hash}` route keeps its own answer: only the query contract
        // moved, so an unknown hash still reports the body the shadowed
        // rejection used to leak into the map route.
        let missing = warp::test::request()
            .path("/api/photos/does-not-exist")
            .reply(&routes)
            .await;

        assert_eq!(missing.status(), 404);
        let body: serde_json::Value = serde_json::from_slice(missing.body()).unwrap();
        assert_eq!(body["error"], "Photo not found");
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
