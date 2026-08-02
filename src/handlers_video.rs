use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::io::SeekFrom;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use warp::http::{HeaderMap, StatusCode};
use warp::{reject, Rejection, Reply};

/// A single, well-formed byte range from a `Range` request header.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ByteRange {
    /// `bytes=start-end` or `bytes=start-` (open-ended end).
    Bounded(u64, Option<u64>),
    /// `bytes=-N`: the last N bytes of the file.
    Suffix(u64),
}

/// Builds a 416 RANGE_NOT_SATISFIABLE response advertising the total size via
/// `Content-Range: bytes */<size>` (RFC 9110 §14.5.1).
fn unsatisfiable_range_response(content_type: &str, file_size: u64) -> Box<dyn Reply> {
    let response = warp::reply::with_status(
        warp::reply::with_header(
            warp::reply::with_header(
                Vec::<u8>::new(),
                "content-range",
                format!("bytes */{}", file_size),
            ),
            "content-type",
            content_type,
        ),
        StatusCode::RANGE_NOT_SATISFIABLE,
    );
    Box::new(response)
}

/// Attach the `X-Transcode-Warning` header only when a transcode attempt
/// actually failed (previously the header was always sent, with an empty
/// value). Boxed so the caller keeps a single `Box<dyn Reply>` return type.
fn with_transcode_warning(
    response: impl Reply + 'static,
    transcoding_failed: bool,
) -> Box<dyn Reply> {
    if transcoding_failed {
        Box::new(warp::reply::with_header(
            response,
            "X-Transcode-Warning",
            "HEVC transcoding not available - serving original video",
        ))
    } else {
        Box::new(response)
    }
}

use crate::db::{DbPool, Photo};
use crate::mimetype_detector;
use crate::video_processor::{
    claim_transcode, get_transcode_status, get_transcoded_path, is_hevc_video,
    set_transcode_status, transcode_hevc_to_h264, TranscodeClaim, TranscodeState, TranscodeStatus,
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
    let (file_to_serve, transcoding_failed) = if client_wants_transcode
        && is_hevc_video(video_path).await.unwrap_or(false)
    {
        log::info!(
            "Client requested transcode for HEVC video: {}",
            photo.filename
        );

        // Get cache directory from environment or use default
        let cache_dir =
            std::env::var("TRANSCODE_CACHE_DIR").unwrap_or_else(|_| "/tmp/turbo-pix".to_string());
        let cache_path = Path::new(&cache_dir);
        let transcoded_path = get_transcoded_path(cache_path, &photo.hash_sha256);

        // Check if transcoded version exists
        if !transcoded_path.exists() {
            // Atomically claim the transcode slot: the claim and the status
            // insert happen under one lock, so two concurrent requests for
            // the same hash cannot both spawn an ffmpeg job (check-then-act
            // race). A previous attempt may have failed/timed out (serve the
            // original instead of re-spawning a doomed 300s job) or still be
            // running (hand back the poll response without starting a
            // second transcode).
            match claim_transcode(&photo.hash_sha256) {
                TranscodeClaim::PreviouslyFailedOrTimedOut => {
                    // A transcode writes to a temp file and renames it into
                    // place only on success, so a failure/timeout leaves no
                    // file at `transcoded_path`. Remove any leftover temp
                    // sibling and serve the original; the warning header
                    // tells the client why.
                    let temp_output_path = transcoded_path.with_extension("mp4.tmp");
                    if temp_output_path.exists() {
                        log::warn!(
                            "Removing leftover temp file from failed/timeout transcode: {}",
                            temp_output_path.display()
                        );
                        let _ = std::fs::remove_file(&temp_output_path);
                    }
                    log::warn!(
                        "Serving original video; previous transcode attempt failed/timed out: {}",
                        photo.filename
                    );
                    (video_path.to_path_buf(), true)
                }
                TranscodeClaim::AlreadyInProgress => {
                    // A transcode spawned by a previous request is still
                    // running: return the poll response without spawning a
                    // second job.
                    log::info!("Transcode already in progress for: {}", photo.filename);
                    let response = warp::reply::with_status(
                        warp::reply::json(&json!({
                            "status": "transcoding",
                            "poll_url": format!("/api/photos/{}/video/status", photo_hash),
                        })),
                        StatusCode::ACCEPTED,
                    );
                    return Ok(Box::new(response));
                }
                TranscodeClaim::Started => {
                    // We own the slot (claim_transcode inserted the
                    // InProgress status): start a fresh transcode.
                    let hash_short = photo.hash_sha256.get(..12).unwrap_or(&photo.hash_sha256);
                    log::info!(
                        "Transcoding HEVC video to H.264: {} (hash: {})",
                        photo.filename,
                        hash_short
                    );

                    let started_at = Utc::now();
                    let hash = photo.hash_sha256.clone();

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
                }
            }
        } else {
            match get_transcode_status(&photo.hash_sha256).map(|s| s.state) {
                Some(TranscodeState::Failed | TranscodeState::Timeout) => {
                    // A previous transcode attempt failed or timed out
                    // mid-write, leaving a corrupt/partial file at the
                    // cache path. Remove it and serve the original instead.
                    log::warn!(
                        "Removing stale transcoded file left by a failed/timeout transcode: {}",
                        transcoded_path.display()
                    );
                    let _ = std::fs::remove_file(&transcoded_path);
                    (video_path.to_path_buf(), true)
                }
                _ => {
                    log::info!(
                        "Using cached transcoded version: {}",
                        transcoded_path.display()
                    );
                    (transcoded_path, false)
                }
            }
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
        Some(range) => {
            // A zero-length file cannot satisfy any byte range: RFC 9110
            // §14.5.1. (A plain GET of an empty file streams below as 200 with
            // content-length 0; 416 applies only to unsatisfiable range
            // requests.)
            if file_size == 0 {
                return Ok(unsatisfiable_range_response(&content_type, 0));
            }

            // Resolve the requested range against the actual file size.
            let (start, end) = match range {
                ByteRange::Suffix(n) => {
                    // RFC 9110: if the suffix length exceeds the representation
                    // size, serve the whole representation.
                    if n >= file_size {
                        (0, file_size - 1)
                    } else {
                        (file_size - n, file_size - 1)
                    }
                }
                ByteRange::Bounded(start, end) => {
                    if start >= file_size {
                        // Range starts past the end of the file: unsatisfiable.
                        return Ok(unsatisfiable_range_response(&content_type, file_size));
                    }
                    let end = end.unwrap_or(file_size - 1).min(file_size - 1);
                    if start > end {
                        return Ok(unsatisfiable_range_response(&content_type, file_size));
                    }
                    (start, end)
                }
            };

            // Stream the requested byte range instead of buffering it in RAM:
            // a full-file range (`bytes=0-`) must not allocate the whole file.
            let mut file = match tokio::fs::File::open(&file_to_serve).await {
                Ok(f) => f,
                Err(_) => return Err(reject::custom(NotFoundError)),
            };

            // The file may have been replaced or shrunk between the stat
            // above and the open; re-stat the open handle so the advertised
            // content-range/content-length match the bytes actually streamed
            // (an over-advertised length truncates the transfer).
            let actual_len = file.metadata().await.map(|m| m.len()).unwrap_or(file_size);
            if start >= actual_len {
                return Ok(unsatisfiable_range_response(&content_type, actual_len));
            }
            let end = end.min(actual_len - 1);
            if start > end {
                return Ok(unsatisfiable_range_response(&content_type, actual_len));
            }

            if file.seek(SeekFrom::Start(start)).await.is_err() {
                return Err(reject::custom(NotFoundError));
            }

            let bytes_to_read = end - start + 1;
            // `take` bounds the stream to exactly the requested range so the
            // body matches the advertised content-length.
            let stream = tokio_util::io::ReaderStream::new(file.take(bytes_to_read));
            let response = warp::reply::stream(stream);
            let response = warp::reply::with_status(response, StatusCode::PARTIAL_CONTENT);
            let response = warp::reply::with_header(response, "content-type", content_type);
            let response = warp::reply::with_header(response, "accept-ranges", "bytes");
            let response = warp::reply::with_header(
                response,
                "content-range",
                format!("bytes {}-{}/{}", start, end, actual_len),
            );
            let response =
                warp::reply::with_header(response, "content-length", bytes_to_read.to_string());
            let response =
                warp::reply::with_header(response, "cache-control", "public, max-age=31536000");

            // Only attach the warning header when transcoding actually failed
            Ok(with_transcode_warning(response, transcoding_failed))
        }
        None => {
            // No range requested: stream the whole file instead of buffering it
            // in RAM. The explicit content-length keeps hyper from switching to
            // chunked transfer encoding.
            let file = match tokio::fs::File::open(&file_to_serve).await {
                Ok(f) => f,
                Err(_) => return Err(reject::custom(NotFoundError)),
            };
            // Re-stat the open handle (the file may have changed since the
            // pre-open stat) so content-length matches the streamed bytes.
            let actual_len = file.metadata().await.map(|m| m.len()).unwrap_or(file_size);
            let stream = tokio_util::io::ReaderStream::new(file);
            let response = warp::reply::stream(stream);
            let response = warp::reply::with_header(response, "content-type", content_type);
            let response =
                warp::reply::with_header(response, "cache-control", "public, max-age=31536000");
            let response = warp::reply::with_header(response, "accept-ranges", "bytes");
            let response =
                warp::reply::with_header(response, "content-length", actual_len.to_string());

            // Only attach the warning header when transcoding actually failed
            Ok(with_transcode_warning(response, transcoding_failed))
        }
    }
}

/// Parse a single-range `Range` header value (e.g. "bytes=0-1023", "bytes=-500").
/// Multi-range values ("bytes=a-b,c-d") and malformed values return `None`, in
/// which case the caller serves the full representation (spec-legal per
/// RFC 9110 §14.2).
fn parse_range_header(value: &str) -> Option<ByteRange> {
    let value = value.strip_prefix("bytes=")?;
    // Multi-range requests are ignored (RFC 9110 allows serving 200 instead).
    if value.contains(',') {
        return None;
    }

    let (start_str, end_str) = value.split_once('-')?;
    if start_str.is_empty() {
        // Suffix range: `bytes=-N` -> the last N bytes.
        let suffix_len = end_str.parse::<u64>().ok()?;
        if suffix_len == 0 {
            // `bytes=-0` is invalid; ignore the range and serve the full body.
            return None;
        }
        return Some(ByteRange::Suffix(suffix_len));
    }

    let start = start_str.parse::<u64>().ok()?;
    let end = if end_str.is_empty() {
        None
    } else {
        Some(end_str.parse::<u64>().ok()?)
    };

    Some(ByteRange::Bounded(start, end))
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
    use crate::video_processor::tests::{acquire_test_env_lock, TestEnvGuard};
    use chrono::Utc;
    use tempfile::TempDir;
    use warp::http::HeaderValue;

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
        _lock: TestEnvGuard,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let lock = acquire_test_env_lock();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                original,
                _lock: lock,
            }
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
        setup_test_video_with_content(db_pool, temp_dir, hash, b"fake-video-data").await
    }

    async fn setup_test_video_with_content(
        db_pool: &DbPool,
        temp_dir: &TempDir,
        hash: &str,
        content: &[u8],
    ) -> std::path::PathBuf {
        let video_path = temp_dir.path().join("video.mp4");
        std::fs::write(&video_path, content).expect("failed to create fake video");

        let photo = Photo {
            hash_sha256: hash.to_string(),
            file_path: video_path.to_str().unwrap().to_string(),
            filename: "video.mp4".to_string(),
            file_size: content.len() as i64,
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

    /// Collect a reply body into a byte vector, driving any streaming body to
    /// completion on the current executor.
    async fn collect_response_body(response: warp::reply::Response) -> Vec<u8> {
        use std::future::poll_fn;
        use std::pin::Pin;
        use warp::hyper::body::Body as _;

        let mut body = response.into_body();
        let mut out = Vec::new();
        while let Some(Ok(frame)) = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
            if let Ok(data) = frame.into_data() {
                out.extend_from_slice(&data);
            }
        }
        out
    }

    #[tokio::test]
    async fn test_video_202() {
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
    async fn test_video_status_transitions() {
        let hash = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        clear_transcode_status(hash);

        set_transcode_status(
            hash,
            TranscodeStatus {
                state: TranscodeState::InProgress,
                hash: hash.to_string(),
                started_at: Some(Utc::now()),
                error: None,
            },
        );

        let response = get_video_status(hash.to_string()).await;
        assert!(response.is_ok(), "status endpoint should return success");

        let in_progress = get_transcode_status(hash).expect("status should be available in store");
        assert_eq!(in_progress.state, TranscodeState::InProgress);

        set_transcode_status(
            hash,
            TranscodeStatus {
                state: TranscodeState::Completed,
                hash: hash.to_string(),
                started_at: in_progress.started_at,
                error: None,
            },
        );

        let completed =
            get_transcode_status(hash).expect("status should still be available in store");
        assert_eq!(completed.state, TranscodeState::Completed);

        clear_transcode_status(hash);
    }

    #[tokio::test]
    async fn test_video_cache_hit() {
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

    #[tokio::test]
    async fn test_video_zero_byte_file() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e1".repeat(32);

        let _video_path = setup_test_video_with_content(&db_pool, &temp_dir, &hash, b"").await;

        // A plain GET of an empty file succeeds with an empty body: 416 applies
        // only to unsatisfiable *range* requests (RFC 9110 §14.5.1).
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            HeaderMap::new(),
            db_pool.clone(),
        )
        .await
        .expect("handler should return a response")
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok()),
            Some("0")
        );
        assert!(response.headers().get("content-range").is_none());

        // A range request against an empty file is unsatisfiable -> 416.
        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=0-"));
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("handler should return a response")
        .into_response();

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok()),
            Some("bytes */0")
        );
        assert!(collect_response_body(response).await.is_empty());
    }

    #[tokio::test]
    async fn test_video_suffix_range_serves_last_bytes() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e2".repeat(32);

        let _video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=-5"));

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("suffix range should succeed")
        .into_response();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok()),
            Some("bytes 10-14/15")
        );
        assert_eq!(
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok()),
            Some("5")
        );
        assert_eq!(collect_response_body(response).await, b"-data".as_slice());
    }

    #[tokio::test]
    async fn test_video_suffix_range_larger_than_file_serves_full() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e3".repeat(32);

        let _video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=-1000"));

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("suffix range should succeed")
        .into_response();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok()),
            Some("bytes 0-14/15")
        );
        assert_eq!(
            collect_response_body(response).await,
            b"fake-video-data".as_slice()
        );
    }

    #[tokio::test]
    async fn test_video_unsatisfiable_range_returns_416() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e4".repeat(32);

        let _video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=100-200"));

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("handler should return a response")
        .into_response();

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok()),
            Some("bytes */15")
        );
        assert!(collect_response_body(response).await.is_empty());
    }

    #[tokio::test]
    async fn test_video_range_start_after_end_returns_416() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e5".repeat(32);

        let _video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=10-5"));

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("handler should return a response")
        .into_response();

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok()),
            Some("bytes */15")
        );
        assert!(collect_response_body(response).await.is_empty());
    }

    #[tokio::test]
    async fn test_video_multi_range_ignored_serves_full() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e6".repeat(32);

        let _video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=0-2,4-6"));

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("handler should return a response")
        .into_response();

        // Multi-range requests are ignored (spec-legal): full 200 response.
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok()),
            Some("15")
        );
        assert!(response.headers().get("content-range").is_none());
    }

    #[tokio::test]
    async fn test_video_full_file_streams_with_content_length() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e7".repeat(32);

        let _video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("handler should stream the file")
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("video/mp4")
        );
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("public, max-age=31536000")
        );
        assert_eq!(
            response
                .headers()
                .get("accept-ranges")
                .and_then(|v| v.to_str().ok()),
            Some("bytes")
        );
        assert_eq!(
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok()),
            Some("15")
        );
        // No failed transcode -> no warning header at all (not even an empty one).
        assert!(response.headers().get("x-transcode-warning").is_none());
    }

    #[tokio::test]
    async fn test_video_stale_transcoded_file_serves_original() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e8".repeat(32);

        let video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'hevc\n'\n");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        // A previous transcode attempt failed, leaving a corrupt file behind.
        let transcoded_path = get_transcoded_path(temp_dir.path(), &hash);
        std::fs::create_dir_all(transcoded_path.parent().unwrap())
            .expect("failed to create cache dir");
        std::fs::write(&transcoded_path, b"partial-corrupt-output")
            .expect("failed to write stale transcoded video");
        set_transcode_status(
            &hash,
            TranscodeStatus {
                state: TranscodeState::Failed,
                hash: hash.clone(),
                started_at: None,
                error: Some("ffmpeg transcode exited with status 1".to_string()),
            },
        );

        let response = get_video_file(
            hash.clone(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("handler should serve the original video")
        .into_response();

        // The stale transcoded file is removed and the original is served with
        // the original MIME type plus a warning header.
        assert!(
            !transcoded_path.exists(),
            "stale transcoded file must be removed"
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("video/mp4")
        );
        assert_eq!(
            response
                .headers()
                .get("x-transcode-warning")
                .and_then(|v| v.to_str().ok()),
            Some("HEVC transcoding not available - serving original video")
        );
        assert_eq!(
            std::fs::read(&video_path).expect("original video should still exist"),
            b"fake-video-data"
        );
        clear_transcode_status(&hash);
    }

    #[tokio::test]
    async fn test_video_full_file_range_streams_with_content_length() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "e9".repeat(32);

        // 1 MiB payload: a full-file range request must stream (206 with the
        // range headers), not buffer the entire file in RAM.
        let content = vec![0xABu8; 1024 * 1024];
        let _video_path = setup_test_video_with_content(&db_pool, &temp_dir, &hash, &content).await;

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=0-"));

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("full-file range should succeed")
        .into_response();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok()),
            Some(format!("bytes 0-{}/{}", content.len() - 1, content.len()).as_str())
        );
        assert_eq!(
            response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok()),
            Some(content.len().to_string().as_str())
        );
        assert_eq!(
            response
                .headers()
                .get("accept-ranges")
                .and_then(|v| v.to_str().ok()),
            Some("bytes")
        );
    }

    #[tokio::test]
    async fn test_video_failed_transcode_without_file_serves_original() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "f1".repeat(32);

        let video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'hevc\n'\n");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        // A previous transcode attempt failed after the temp+rename change, so
        // no file exists at the cache path -- only a leftover temp sibling.
        let transcoded_path = get_transcoded_path(temp_dir.path(), &hash);
        std::fs::create_dir_all(transcoded_path.parent().unwrap())
            .expect("failed to create cache dir");
        let temp_output_path = transcoded_path.with_extension("mp4.tmp");
        std::fs::write(&temp_output_path, b"partial-output")
            .expect("failed to write leftover temp file");
        set_transcode_status(
            &hash,
            TranscodeStatus {
                state: TranscodeState::Failed,
                hash: hash.clone(),
                started_at: None,
                error: Some("ffmpeg transcode exited with status 1".to_string()),
            },
        );

        let response = get_video_file(
            hash.clone(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("handler should serve the original video")
        .into_response();

        // No 202 re-spawn: the original is served with the original MIME type
        // plus a warning header, and the leftover temp file is removed.
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            !temp_output_path.exists(),
            "leftover temp file must be removed"
        );
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("video/mp4")
        );
        assert_eq!(
            response
                .headers()
                .get("x-transcode-warning")
                .and_then(|v| v.to_str().ok()),
            Some("HEVC transcoding not available - serving original video")
        );
        assert_eq!(
            std::fs::read(&video_path).expect("original video should still exist"),
            b"fake-video-data"
        );
        clear_transcode_status(&hash);
    }

    #[tokio::test]
    async fn test_video_in_progress_transcode_returns_202_without_respawn() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "f2".repeat(32);

        let _video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'hevc\n'\n");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        // A transcode spawned by a previous request is already running and no
        // output file exists yet.
        let started_at = Utc::now();
        set_transcode_status(
            &hash,
            TranscodeStatus {
                state: TranscodeState::InProgress,
                hash: hash.clone(),
                started_at: Some(started_at),
                error: None,
            },
        );

        let response = get_video_file(
            hash.clone(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("handler should return the poll response")
        .into_response();

        assert_eq!(response.status(), StatusCode::ACCEPTED);

        // The status entry is untouched: no second job was spawned to replace
        // it (a respawn would reset started_at to a fresh timestamp).
        let status = get_transcode_status(&hash).expect("status should still be present");
        assert_eq!(status.state, TranscodeState::InProgress);
        assert_eq!(status.started_at, Some(started_at));
        assert!(
            !get_transcoded_path(temp_dir.path(), &hash).exists(),
            "no transcode output should exist"
        );

        clear_transcode_status(&hash);
    }
}
