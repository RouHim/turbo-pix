use serde::Deserialize;
use serde_json::json;
use std::path::Path;
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
                mimetype_detector::from_path(Path::new(&photo.file_path))
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "application/octet-stream".to_string())
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
