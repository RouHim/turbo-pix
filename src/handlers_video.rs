use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use warp::http::{HeaderMap, StatusCode};
use warp::{reject, Rejection, Reply};

use crate::db::{DbPool, Photo};
use crate::mimetype_detector;
use crate::warp_helpers::{DatabaseError, NotFoundError};

#[derive(Debug, Deserialize)]
pub struct VideoQuery {
    pub metadata: Option<String>,
}

pub async fn get_video_file(
    photo_hash: String,
    query: VideoQuery,
    headers: HeaderMap,
    db_pool: DbPool,
) -> Result<Box<dyn Reply>, Rejection> {
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash) {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            log::error!("Database error: {}", e);
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
            "video_codec": photo.video_codec(),
            "audio_codec": photo.audio_codec(),
            "bitrate": photo.bitrate(),
            "frame_rate": photo.frame_rate(),
            "width": photo.width,
            "height": photo.height,
            "taken_at": photo.taken_at.map(|dt| dt.to_rfc3339()),
            "file_path": photo.file_path,
        });

        return Ok(Box::new(warp::reply::json(&video_metadata)));
    }

    // Get file metadata
    let file_metadata = match std::fs::metadata(&photo.file_path) {
        Ok(metadata) => metadata,
        Err(_) => return Err(reject::custom(NotFoundError)),
    };

    let file_size = file_metadata.len();
    let content_type = photo.mime_type.unwrap_or_else(|| {
        mimetype_detector::from_path(Path::new(&photo.file_path))
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string())
    });

    // Parse Range header
    let range_header = headers
        .get("range")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range_header);

    match range_header {
        Some((start, end)) => {
            // Validate and adjust range
            let start = start.min(file_size - 1);
            let end = end.unwrap_or(file_size - 1).min(file_size - 1);

            if start > end {
                return Err(reject::custom(NotFoundError));
            }

            // Read the requested byte range
            let mut file = match File::open(&photo.file_path) {
                Ok(f) => f,
                Err(_) => return Err(reject::custom(NotFoundError)),
            };

            if file.seek(SeekFrom::Start(start)).is_err() {
                return Err(reject::custom(NotFoundError));
            }

            let bytes_to_read = (end - start + 1) as usize;
            let mut buffer = vec![0u8; bytes_to_read];

            match file.read_exact(&mut buffer) {
                Ok(_) => {
                    let response = warp::reply::with_status(buffer, StatusCode::PARTIAL_CONTENT);
                    let response = warp::reply::with_header(response, "content-type", content_type);
                    let response = warp::reply::with_header(response, "accept-ranges", "bytes");
                    let response = warp::reply::with_header(
                        response,
                        "content-range",
                        format!("bytes {}-{}/{}", start, end, file_size),
                    );
                    let response = warp::reply::with_header(
                        response,
                        "content-length",
                        bytes_to_read.to_string(),
                    );
                    let response = warp::reply::with_header(
                        response,
                        "cache-control",
                        "public, max-age=31536000",
                    );

                    Ok(Box::new(response))
                }
                Err(_) => Err(reject::custom(NotFoundError)),
            }
        }
        None => {
            // No range requested, send entire file
            match std::fs::read(&photo.file_path) {
                Ok(file_data) => {
                    let response =
                        warp::reply::with_header(file_data, "content-type", content_type);
                    let response = warp::reply::with_header(
                        response,
                        "cache-control",
                        "public, max-age=31536000",
                    );
                    let response = warp::reply::with_header(response, "accept-ranges", "bytes");
                    let response =
                        warp::reply::with_header(response, "content-length", file_size.to_string());

                    Ok(Box::new(response))
                }
                Err(_) => Err(reject::custom(NotFoundError)),
            }
        }
    }
}

/// Parse the Range header value (e.g., "bytes=0-1023")
/// Returns (start, Option<end>)
fn parse_range_header(value: &str) -> Option<(u64, Option<u64>)> {
    let value = value.strip_prefix("bytes=")?;
    let parts: Vec<&str> = value.split('-').collect();

    if parts.len() != 2 {
        return None;
    }

    let start = parts[0].parse::<u64>().ok()?;
    let end = if parts[1].is_empty() {
        None
    } else {
        parts[1].parse::<u64>().ok()
    };

    Some((start, end))
}
