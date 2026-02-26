use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use warp::http::{HeaderMap, StatusCode};
use warp::{reject, Rejection, Reply};

use crate::db::{DbPool, Photo};
use crate::mimetype_detector;
use crate::video_processor::{
    get_transcode_status, get_transcoded_path, is_hevc_video, set_transcode_status,
    transcode_hevc_to_h264, TranscodeState, TranscodeStatus,
};
use crate::warp_helpers::{DatabaseError, NotFoundError};

#[derive(Debug, Deserialize)]
pub struct VideoQuery {
    pub metadata: Option<String>,
    pub transcode: Option<String>,
}

pub async fn get_video_file(
    photo_hash: String,
    query: VideoQuery,
    headers: HeaderMap,
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

    // Check if client explicitly requested transcoding
    let client_wants_transcode = query
        .transcode
        .as_ref()
        .map(|v| v == "true")
        .unwrap_or(false);

    // Determine which file to serve (original or transcoded)
    let video_path = Path::new(&photo.file_path);
    let (file_to_serve, transcoding_failed) =
        if client_wants_transcode && is_hevc_video(video_path).await.unwrap_or(false) {
            log::info!(
                "Client requested transcode for HEVC video: {}",
                photo.filename
            );

            // Get cache directory from environment or use default
            let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
                .unwrap_or_else(|_| "/tmp/turbo-pix".to_string());
            let cache_path = Path::new(&cache_dir);
            let transcoded_path = get_transcoded_path(cache_path, &photo.hash_sha256);

            // Check if transcoded version exists
            if !transcoded_path.exists() {
                log::info!(
                    "Transcoding HEVC video to H.264: {} (hash: {})",
                    photo.filename,
                    &photo.hash_sha256[..12]
                );

                let started_at = Utc::now();
                let hash = photo.hash_sha256.clone();
                set_transcode_status(
                    &hash,
                    TranscodeStatus {
                        state: TranscodeState::InProgress,
                        hash: hash.clone(),
                        started_at: Some(started_at),
                        error: None,
                    },
                );

                let input_path = video_path.to_path_buf();
                let output_path = transcoded_path.clone();
                tokio::spawn(async move {
                    match transcode_hevc_to_h264(&input_path, &output_path).await {
                        Ok(_) => {
                            set_transcode_status(
                                &hash,
                                TranscodeStatus {
                                    state: TranscodeState::Completed,
                                    hash: hash.clone(),
                                    started_at: Some(started_at),
                                    error: None,
                                },
                            );
                        }
                        Err(e) => {
                            let error = e.to_string();
                            let state = if error.to_ascii_lowercase().contains("timed out") {
                                TranscodeState::Timeout
                            } else {
                                TranscodeState::Failed
                            };

                            set_transcode_status(
                                &hash,
                                TranscodeStatus {
                                    state,
                                    hash: hash.clone(),
                                    started_at: Some(started_at),
                                    error: Some(error),
                                },
                            );
                        }
                    }
                });

                let response = warp::reply::with_status(
                    warp::reply::json(&json!({
                        "status": "transcoding",
                        "poll_url": format!("/api/photos/{}/video/status", photo_hash),
                    })),
                    StatusCode::ACCEPTED,
                );
                return Ok(Box::new(response));
            } else {
                log::info!(
                    "Using cached transcoded version: {}",
                    transcoded_path.display()
                );
                (transcoded_path, false)
            }
        } else {
            // Serve original video (client supports HEVC or video is not HEVC)
            if client_wants_transcode {
                log::info!(
                    "Transcode requested but video is not HEVC, serving original: {}",
                    photo.filename
                );
            }
            (video_path.to_path_buf(), false)
        };

    // Get file metadata
    let file_metadata = match std::fs::metadata(&file_to_serve) {
        Ok(metadata) => metadata,
        Err(_) => return Err(reject::custom(NotFoundError)),
    };

    let file_size = file_metadata.len();

    // Determine correct MIME type based on whether we're serving transcoded content
    let content_type =
        if client_wants_transcode && file_to_serve != video_path && !transcoding_failed {
            // Serving transcoded H.264 video - always use video/mp4
            "video/mp4".to_string()
        } else {
            // Serving original video - use stored/detected MIME type
            photo.mime_type.unwrap_or_else(|| {
                mimetype_detector::from_path(Path::new(&photo.file_path))
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "application/octet-stream".to_string())
            })
        };

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
            let mut file = match File::open(&file_to_serve) {
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

                    // Add warning header if transcoding failed
                    let warning_value = if transcoding_failed {
                        "HEVC transcoding not available - serving original video"
                    } else {
                        ""
                    };
                    let response =
                        warp::reply::with_header(response, "X-Transcode-Warning", warning_value);

                    Ok(Box::new(response))
                }
                Err(_) => Err(reject::custom(NotFoundError)),
            }
        }
        None => {
            // No range requested, send entire file
            match std::fs::read(&file_to_serve) {
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

                    // Add warning header if transcoding failed
                    let warning_value = if transcoding_failed {
                        "HEVC transcoding not available - serving original video"
                    } else {
                        ""
                    };
                    let response =
                        warp::reply::with_header(response, "X-Transcode-Warning", warning_value);

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

pub async fn get_video_status(photo_hash: String) -> Result<impl Reply, Rejection> {
    match get_transcode_status(&photo_hash) {
        Some(status) => Ok(warp::reply::json(&status)),
        None => Err(reject::custom(NotFoundError)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_in_memory_pool;
    use crate::video_processor::clear_transcode_status;
    use chrono::Utc;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

    fn test_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = &self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    fn create_script(path: &Path, content: &str) {
        std::fs::write(path, content).expect("failed to write script");
        make_executable(path);
    }

    async fn setup_test_video(
        db_pool: &DbPool,
        temp_dir: &TempDir,
        hash: &str,
    ) -> std::path::PathBuf {
        let video_path = temp_dir.path().join("video.mp4");
        std::fs::write(&video_path, b"fake-video-data").expect("failed to create fake video");

        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: video_path.to_str().unwrap().to_string(),
            filename: "video.mp4".to_string(),
            file_size: 15,
            mime_type: Some("video/mp4".to_string()),
            taken_at: None,
            width: Some(1920),
            height: Some(1080),
            orientation: None,
            duration: Some(1.0),
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
            .expect("failed to create test photo entry");

        video_path
    }

    #[tokio::test]
    async fn test_video_202() {
        let _lock = test_env_lock().lock().unwrap();
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        clear_transcode_status(hash);

        let _video_path = setup_test_video(&db_pool, &temp_dir, hash).await;

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'hevc\n'\n");

        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg.sh");
        create_script(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nsleep 1\nfor last; do :; done\nmkdir -p \"$(dirname \"$last\")\"\ntouch \"$last\"\n",
        );

        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("handler should return accepted response")
        .into_response();

        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let transcode_status = get_transcode_status(hash).expect("status should be set");
        assert_eq!(transcode_status.state, TranscodeState::InProgress);

        clear_transcode_status(hash);
    }

    #[tokio::test]
    async fn test_video_status_poll() {
        let hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        clear_transcode_status(hash);
        let expected = TranscodeStatus {
            state: TranscodeState::Completed,
            hash: hash.to_string(),
            started_at: Some(Utc::now()),
            error: None,
        };
        set_transcode_status(hash, expected.clone());

        let response = get_video_status(hash.to_string()).await;
        assert!(response.is_ok(), "status endpoint should return success");
        assert_eq!(response.unwrap().into_response().status(), StatusCode::OK);

        let status = get_transcode_status(hash).expect("status should be available in store");
        assert_eq!(status.state, TranscodeState::Completed);
        assert_eq!(status.hash, expected.hash);

        clear_transcode_status(hash);
    }

    #[tokio::test]
    async fn test_video_cache_hit() {
        let _lock = test_env_lock().lock().unwrap();
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

        let _video_path = setup_test_video(&db_pool, &temp_dir, hash).await;

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'hevc\n'\n");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let transcoded_path = get_transcoded_path(temp_dir.path(), hash);
        std::fs::create_dir_all(transcoded_path.parent().unwrap())
            .expect("failed to create cache dir");
        std::fs::write(&transcoded_path, b"cached-transcoded-video")
            .expect("failed to write cached transcoded video");

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("cache hit should succeed")
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("video/mp4")
        );
    }

    #[tokio::test]
    async fn test_video_status_404() {
        let hash = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        clear_transcode_status(hash);

        let result = get_video_status(hash.to_string()).await;
        match result {
            Ok(_) => panic!("expected missing hash to return NotFoundError"),
            Err(rejection) => {
                assert!(
                    rejection.find::<NotFoundError>().is_some(),
                    "expected NotFoundError rejection"
                );
            }
        }
    }
}
