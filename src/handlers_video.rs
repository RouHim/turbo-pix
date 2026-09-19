use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
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

/// Attach the `X-Transcode-Warning` header only when a transcode/remux attempt
/// actually failed (`warning` is `Some(message)`). Previously the header was
/// always sent, with an empty value. Boxed so the caller keeps a single
/// `Box<dyn Reply>` return type.
fn with_transcode_warning(
    response: impl Reply + 'static,
    warning: Option<&'static str>,
) -> Box<dyn Reply> {
    match warning {
        Some(message) => Box::new(warp::reply::with_header(
            response,
            "X-Transcode-Warning",
            message,
        )),
        None => Box::new(response),
    }
}
/// Warning sent when a transcode attempt failed and the original is served.
const TRANSCODE_FAILED_WARNING: &str = "HEVC transcoding not available - serving original video";
/// Warning sent when the transcode worker pool is saturated and the original
/// is served instead of queueing.
const TRANSCODE_BUSY_WARNING: &str = "Transcode worker pool busy - serving original video";

use std::sync::Arc;

use crate::db::{DbPool, Photo};
use crate::mimetype_detector;
use crate::video_capability::{plan, ClientCodecs, Delivery};
use crate::video_processor::{
    claim_transcode, convert_video_with_progress, get_transcode_status,
    get_transcoded_path_versioned, remux_to_faststart_mp4, set_transcode_status, FileConversion,
    TranscodeClaim, TranscodeState, TranscodeStatus,
};
use crate::video_stream::{
    output_mime, start_stream, supervise, StreamHandle, StreamMode, StreamStartError,
    FULL_RUN_MAX_START_SECS,
};
use crate::warp_helpers::{DatabaseError, NotFoundError};

#[derive(Debug, Deserialize)]
pub struct VideoQuery {
    pub metadata: Option<String>,
    pub transcode: Option<String>,
    /// Client codec capabilities, from the `?client=` query param (read into this
    /// field via serde rename). The request header `X-TurboPix-Codecs` takes
    /// precedence over this at decision time.
    #[serde(rename = "client")]
    pub client_codecs: Option<String>,
    /// `?decision` (bare or `=true`) → return a JSON playback decision
    /// ({action,url,reason}) instead of streaming, so the client can choose
    /// Direct Play / remux / transcode without probing the stream itself.
    pub decision: Option<String>,
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
            // `file_path` is deliberately omitted: it is an absolute server
            // path and disclosing it in the metadata payload leaks the host's
            // filesystem layout to clients.
        });

        return Ok(Box::new(warp::reply::json(&video_metadata)));
    }

    // Check if client explicitly requested transcoding
    let client_wants_transcode = query
        .transcode
        .as_ref()
        .map(|v| v == "true")
        .unwrap_or(false);

    // Resolve the client's declared codecs: the request header (clients that
    // can set one) takes precedence over the `?client=` query param (the
    // browser's media requests, which cannot). The same string is echoed into
    // the returned URLs so the follow-up serve-time re-decision sees the same
    // declaration. Neither present → conservative (h264-8, no audio).
    let client_param = headers
        .get("X-TurboPix-Codecs")
        .and_then(|v| v.to_str().ok())
        .or(query.client_codecs.as_deref())
        .unwrap_or_default();
    let client = ClientCodecs::parse(Some(client_param));

    let video_path = Path::new(&photo.file_path);

    // Stat the backing file BEFORE the serve-time decision: a 0-byte file (a
    // sync app's `.pending-*` temp) can never be played, remuxed, or
    // transcoded, so fast-fail with an empty warning and, crucially, WITHOUT
    // claiming a transcode slot. A missing backing file → 404.
    let source_size = match std::fs::metadata(video_path) {
        Ok(metadata) => metadata.len(),
        Err(_) => return Err(reject::custom(NotFoundError)),
    };
    if source_size == 0 {
        log::warn!(
            "Serving empty video file (0 bytes, likely a pending temp): {}",
            photo.filename
        );
        // A `?decision` probe on an empty file reports the empty action
        // (the client surfaces "empty / still syncing") rather than a blank
        // 200 the JSON parser would choke on.
        if query
            .decision
            .as_deref()
            .is_some_and(|v| v.is_empty() || v == "true")
        {
            let response = json!({
                "action": "empty",
                "url": null,
                "mode": null,
                "mime": null,
                "duration": null,
                "cached": false,
                "reason": null,
            });
            return Ok(Box::new(warp::reply::json(&response)));
        }
        let response = warp::reply::with_status(Vec::<u8>::new(), StatusCode::OK);
        let response = warp::reply::with_header(response, "content-length", "0");
        let response = warp::reply::with_header(response, "x-transcode-warning", "empty");
        return Ok(Box::new(response));
    }

    // Capability record resolution (Task 1) plus the single decision engine:
    // missing container / bit-depth / layout facts are derived from the file
    // and persisted, so an incomplete legacy record cannot force a conversion.
    let caps = crate::video_probe::resolve(&db_pool, &photo).await;
    let delivery = plan(&caps, &client);

    let video_url = if client_param.is_empty() {
        format!("/api/photos/{}/video", photo_hash)
    } else {
        format!(
            "/api/photos/{}/video?client={}",
            photo_hash,
            urlencoding(client_param)
        )
    };
    let stream_base = if client_param.is_empty() {
        format!("/api/photos/{}/video/stream", photo_hash)
    } else {
        format!(
            "/api/photos/{}/video/stream?client={}",
            photo_hash,
            urlencoding(client_param)
        )
    };
    let mode = match delivery {
        Delivery::Direct => None,
        Delivery::StreamRemux => Some(StreamMode::Remux),
        Delivery::StreamAudio => Some(StreamMode::Audio),
        Delivery::StreamTranscode => Some(StreamMode::Transcode),
    };

    // Cache fast path (FR-010): an artifact already produced for this exact
    // source version is played as bytes, exactly like the original, instead of
    // converting the same file a second time. Absence never gates playback —
    // an uncached video simply gets the `stream` decision below.
    let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
        .unwrap_or_else(|_| "./data/cache/transcoded".to_string());
    let whole_file_artifact = get_transcoded_path_versioned(
        Path::new(&cache_dir),
        &photo.hash_sha256,
        photo.file_size,
        photo.date_modified.timestamp_millis(),
    );
    // A file left behind by a failed/timed-out attempt is not a usable
    // artifact: the serve path deletes it and falls back to the original, so
    // the decision must not advertise it as cached either.
    let cached_whole_file = whole_file_artifact.exists()
        && !matches!(
            get_transcode_status(&photo.hash_sha256).map(|status| status.state),
            Some(TranscodeState::Failed | TranscodeState::Timeout)
        );
    // The faststart sidecar is the remux cache: a `-c copy` of the source, so
    // when it is there the remux run has nothing left to produce.
    let remux_sidecar = remux_sidecar_path(
        &cache_dir,
        &photo.hash_sha256,
        photo.file_size,
        photo.date_modified.timestamp_millis(),
    );
    let cached_remux = matches!(delivery, Delivery::StreamRemux) && remux_sidecar.exists();

    // `?decision` (bare or `=true`): don't stream — return the recommended
    // playback action as JSON so the client can pick without probing the
    // stream itself. A `stream` decision's `url` carries NEITHER `start` nor
    // `mode`: the player appends `start=<seconds>` on every (re)start and
    // `mode=<decision.mode>`, which is the only mode the server authorized.
    if query
        .decision
        .as_deref()
        .is_some_and(|v| v.is_empty() || v == "true")
    {
        let response = match delivery {
            Delivery::Direct => json!({
                "action": "direct",
                "url": video_url,
                "mode": null,
                "mime": null,
                "duration": caps.duration_secs,
                "cached": false,
                "reason": null,
            }),
            // A completed conversion is served as a file: `?transcode=true`
            // routes the byte request to the cache (a plain byte request would
            // re-decide and could hand back the source codec the client cannot
            // play).
            _ if cached_whole_file => json!({
                "action": "direct",
                "url": format!(
                    "{video_url}{}transcode=true",
                    if video_url.contains('?') { "&" } else { "?" }
                ),
                "mode": null,
                "mime": null,
                "duration": caps.duration_secs,
                "cached": true,
                "reason": null,
            }),
            // Same for a cached faststart sidecar: the source only needed its
            // moov moved, and that copy already exists.
            _ if cached_remux => json!({
                "action": "direct",
                "url": video_url,
                "mode": null,
                "mime": null,
                "duration": caps.duration_secs,
                "cached": true,
                "reason": null,
            }),
            _ => {
                let mode = mode.expect("stream deliveries carry a mode");
                json!({
                    "action": "stream",
                    "url": stream_base,
                    "mode": mode.as_str(),
                    "mime": output_mime(mode, &caps.codec, caps.audio_codec.as_deref()),
                    "duration": caps.duration_secs,
                    "cached": false,
                    "reason": null,
                })
            }
        };
        return Ok(Box::new(warp::reply::json(&response)));
    }

    // Decide which file to serve for a byte request: the original (Direct),
    // a faststart remux sidecar when one is already cached, or the whole-file
    // conversion escape hatch. `warning` is Some(reason) only when a
    // conversion attempt failed and we fell back to the original.
    let (file_to_serve, warning) = match delivery {
        Delivery::Direct => {
            if client_wants_transcode {
                log::info!(
                    "Transcode requested but video is directly playable, serving original: {}",
                    photo.filename
                );
            }
            (video_path.to_path_buf(), None)
        }
        Delivery::StreamRemux => {
            // A cached faststart sidecar is a lossless, immediately playable
            // copy — always better than converting the whole file.
            if cached_remux {
                (remux_sidecar, None)
            } else if client_wants_transcode {
                return serve_whole_file_transcode(&photo, &headers).await;
            } else {
                (video_path.to_path_buf(), None)
            }
        }
        Delivery::StreamAudio | Delivery::StreamTranscode => {
            if client_wants_transcode {
                // Escape hatch for clients that cannot consume the stream
                // (no MSE for the delivered codec) and for explicit retries.
                return serve_whole_file_transcode(&photo, &headers).await;
            }
            // A byte request that is not the stream endpoint means the client
            // wants a file; serve the original and let it decide.
            (video_path.to_path_buf(), None)
        }
    };

    serve_video_file(&photo, video_path, file_to_serve, warning, &headers).await
}

/// Serve one video file: its MIME type, range handling, and streamed body,
/// with the optional `X-Transcode-Warning` header. `source_path` is the
/// ORIGINAL file, which is what distinguishes "the original's own MIME type"
/// from the MP4 every derived artifact is.
async fn serve_video_file(
    photo: &Photo,
    source_path: &Path,
    file_to_serve: std::path::PathBuf,
    warning: Option<&'static str>,
    headers: &HeaderMap,
) -> Result<Box<dyn Reply>, Rejection> {
    // Get file metadata
    let file_metadata = match std::fs::metadata(&file_to_serve) {
        Ok(metadata) => metadata,
        Err(_) => return Err(reject::custom(NotFoundError)),
    };

    let file_size = file_metadata.len();

    // Determine correct MIME type based on what's being served: both the
    // faststart remux sidecar and the transcoded output are MP4 (a stream copy
    // of an MP4, or an H.264 transcode), so serve `video/mp4` for them. The
    // original uses its stored/detected MIME type.
    let content_type = if file_to_serve.as_path() != source_path {
        "video/mp4".to_string()
    } else {
        photo.mime_type.clone().unwrap_or_else(|| {
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
            Ok(with_transcode_warning(response, warning))
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
            Ok(with_transcode_warning(response, warning))
        }
    }
}
/// Percent-encode the capability string for a query string.
///
/// Capability tokens are `[a-z0-9,-]`, so escaping the comma is enough to keep
/// the value unambiguous without pulling in a URL-encoding dependency.
fn urlencoding(value: &str) -> String {
    value.replace(',', "%2C").replace(' ', "")
}

/// Whole-file conversion escape hatch: the legacy `?transcode=true` flow
/// (claim a conversion slot, spawn the H.264 conversion, answer 202 + poll URL,
/// or serve the completed cache artifact). The streaming path
/// (`/video/stream`) is the normal delivery; this remains for clients that
/// cannot consume the streamed codec and for explicit user retries.
async fn serve_whole_file_transcode(
    photo: &Photo,
    headers: &HeaderMap,
) -> Result<Box<dyn Reply>, Rejection> {
    let video_path = Path::new(&photo.file_path);
    let record_codec = photo.video_codec().unwrap_or("");

    log::info!(
        "Client requested transcode for video (codec: {}): {}",
        record_codec,
        photo.filename
    );

    // Get cache directory from environment or use the app data path (not /tmp/turbo-pix: a
    // predictable world-writable path is squat-able by local users via symlinks). main.rs defaults
    // the env var from config when unset.
    let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
        .unwrap_or_else(|_| "./data/cache/transcoded".to_string());
    let cache_path = Path::new(&cache_dir);
    // Versioned by the source's size+mtime (the DB hash is path-derived, so an in-place edit keeps
    // the hash while the bytes change — the version makes this miss after the rescan notices the
    // edit instead of serving the stale H.264 transcode forever).
    let transcoded_path = get_transcoded_path_versioned(
        cache_path,
        &photo.hash_sha256,
        photo.file_size,
        photo.date_modified.timestamp_millis(),
    );

    // Check if transcoded version exists
    if !transcoded_path.exists() {
        // Atomically claim the transcode slot: the claim and the status insert happen under one
        // lock, so two concurrent requests for the same hash cannot both spawn an ffmpeg job
        // (check-then-act race). A previous attempt may have failed/timed out (serve the original
        // instead of re-spawning a doomed 300s job) or still be running (hand back the poll
        // response without starting a second transcode).
        match claim_transcode(&photo.hash_sha256) {
            TranscodeClaim::PreviouslyFailedOrTimedOut => {
                // A transcode writes to a temp file and renames it into place only on success, so a
                // failure/timeout leaves no file at `transcoded_path`. Remove any leftover temp
                // sibling and serve the original; the warning header tells the client why.
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
                serve_video_file(
                    photo,
                    video_path,
                    video_path.to_path_buf(),
                    Some(TRANSCODE_FAILED_WARNING),
                    headers,
                )
                .await
            }
            TranscodeClaim::AlreadyInProgress => {
                // A transcode spawned by a previous request is still running: return the poll
                // response without spawning a second job.
                log::info!("Transcode already in progress for: {}", photo.filename);
                let response = warp::reply::with_status(
                    warp::reply::json(&json!({
                        "status": "transcoding",
                        "poll_url": format!("/api/photos/{}/video/status", photo.hash_sha256),
                    })),
                    StatusCode::ACCEPTED,
                );
                Ok(Box::new(response))
            }
            TranscodeClaim::PoolSaturated => {
                // The worker pool is at its concurrent-claim cap (or transcoding is disabled):
                // serve the original instead of queueing an unbounded spawned task.
                log::warn!(
                    "Transcode pool saturated; serving original: {}",
                    photo.filename
                );
                serve_video_file(
                    photo,
                    video_path,
                    video_path.to_path_buf(),
                    Some(TRANSCODE_BUSY_WARNING),
                    headers,
                )
                .await
            }
            TranscodeClaim::Started => {
                // We own the slot (claim_transcode inserted the InProgress status): start a fresh
                // transcode.
                // The escape hatch exists for clients that cannot consume the
                // stream at all, so its artifact must play everywhere: a full
                // re-encode, never a video copy.
                spawn_whole_file_transcode(
                    photo,
                    transcoded_path.clone(),
                    FileConversion::Reencode,
                    photo.audio_codec(),
                );

                let response = warp::reply::with_status(
                    warp::reply::json(&json!({
                        "status": "transcoding",
                        "poll_url": format!("/api/photos/{}/video/status", photo.hash_sha256),
                    })),
                    StatusCode::ACCEPTED,
                );
                Ok(Box::new(response))
            }
        }
    } else {
        match get_transcode_status(&photo.hash_sha256).map(|s| s.state) {
            Some(TranscodeState::Failed | TranscodeState::Timeout) => {
                // A previous transcode attempt failed or timed out mid-write, leaving a
                // corrupt/partial file at the cache path. Remove it and serve the original instead.
                log::warn!(
                    "Removing stale transcoded file left by a failed/timeout transcode: {}",
                    transcoded_path.display()
                );
                let _ = std::fs::remove_file(&transcoded_path);
                serve_video_file(
                    photo,
                    video_path,
                    video_path.to_path_buf(),
                    Some(TRANSCODE_FAILED_WARNING),
                    headers,
                )
                .await
            }
            _ => {
                log::info!(
                    "Using cached transcoded version: {}",
                    transcoded_path.display()
                );
                serve_video_file(photo, video_path, transcoded_path, None, headers).await
            }
        }
    }
}

/// Spawn a whole-file conversion of `photo` into `output_path` in the
/// background, keeping the transcode status the poll endpoint reports up to
/// date while it runs. The caller owns the claim (see [`claim_transcode`]), so
/// two encoders for one hash can never overlap.
///
/// `conversion` picks the video handling and `audio_codec` (the source's audio
/// codec as best known: the resolved capabilities where they are at hand, the
/// stored record otherwise) decides whether the audio track is copied or
/// converted — see [`crate::video_processor::build_conversion_args`].
fn spawn_whole_file_transcode(
    photo: &Photo,
    output_path: PathBuf,
    conversion: FileConversion,
    audio_codec: Option<&str>,
) {
    let hash = photo.hash_sha256.clone();
    let hash_short = hash.get(..12).unwrap_or(&hash).to_string();
    let input_path = PathBuf::from(&photo.file_path);
    let audio_codec = audio_codec.map(str::to_string);
    log::info!(
        "Converting video ({}): {} (hash: {})",
        if conversion == FileConversion::VideoCopy {
            "video copy"
        } else {
            "re-encode to H.264"
        },
        photo.filename,
        hash_short
    );

    let started_at = Utc::now();
    let hash_for_progress = hash.clone();
    let on_progress = {
        let hash_for_progress = hash_for_progress.clone();
        Arc::new(move |percent: Option<u8>| {
            set_transcode_status(
                &hash_for_progress,
                TranscodeStatus {
                    state: TranscodeState::InProgress,
                    hash: hash_for_progress.clone(),
                    started_at: Some(started_at),
                    error: None,
                    percent,
                },
            );
        })
    };
    tokio::spawn(async move {
        match convert_video_with_progress(
            &input_path,
            &output_path,
            conversion,
            audio_codec.as_deref(),
            on_progress,
        )
        .await
        {
            Ok(_) => {
                // Only one transcode version file per hash: remove older `{hash}_*.mp4`
                // siblings now that the new version is in place (the versioned name
                // folds in size+mtime, so an in-place edit produces a new file rather
                // than overwriting).
                if let Some(parent) = output_path.parent() {
                    if let Ok(entries) = std::fs::read_dir(parent) {
                        let new_name = output_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default();
                        for entry in entries.filter_map(|e| e.ok()) {
                            let path = entry.path();
                            let is_old_version =
                                path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                                    n.starts_with(&format!("{}_", hash))
                                        && n.ends_with(".mp4")
                                        && n != new_name
                                });
                            if is_old_version {
                                let _ = std::fs::remove_file(&path);
                            }
                        }
                    }
                }
                set_transcode_status(
                    &hash,
                    TranscodeStatus {
                        state: TranscodeState::Completed,
                        hash: hash.clone(),
                        started_at: Some(started_at),
                        error: None,
                        percent: Some(100),
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
                        percent: None,
                    },
                );
            }
        }
    });
}

/// Fill the cache after a successful full playthrough, so the next open starts
/// like a native play (FR-010, SC-007): the work has been paid for once
/// already, and paying for it again on every open is waste.
///
/// What gets cached is what the run itself produced, so the artifact is never
/// weaker than the stream it replaces:
/// - `StreamTranscode` (the client cannot decode the source video) → the
///   whole-file H.264 + AAC conversion.
/// - `StreamAudio` (the client decodes the video, not the audio) → the same
///   file with the video track *copied* and only the audio converted.
/// - `StreamRemux` (container/layout only) → the lossless faststart sidecar
///   under `{TRANSCODE_CACHE_DIR}/remux/`, i.e. exactly the path the playback
///   decision reuses for a moov-at-end source. A re-encode here would degrade
///   video the client already plays.
///
/// Runs after the stream the client was playing has ended — it can never gate
/// first playback — and is bounded by the same claim + worker pool (or the
/// remux semaphore for the sidecar) as every other conversion, so a hash that
/// is already converting (or whose attempt failed within the retry cooldown) is
/// left alone instead of queueing a second job.
fn spawn_cache_fill(photo: &Photo, mode: StreamMode, audio_codec: Option<&str>) {
    let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
        .unwrap_or_else(|_| "./data/cache/transcoded".to_string());

    if mode == StreamMode::Remux {
        let sidecar = remux_sidecar_path(
            &cache_dir,
            &photo.hash_sha256,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        if sidecar.exists() {
            return;
        }
        // The unconditional remux, not `ensure_progressive_mp4`: a finished
        // remux run already proved the source cannot be played as it is (a
        // Matroska has no moov for the serve-time probe to find, and its sidecar
        // is the whole point). The core re-checks existence under the remux
        // semaphore and atomically renames a unique temp file into place, so
        // concurrent fills cannot interleave.
        let source = PathBuf::from(&photo.file_path);
        let hash = photo.hash_sha256.clone();
        tokio::spawn(async move {
            match remux_to_faststart_mp4(&source, &sidecar).await {
                Ok(()) => log::info!("Remux cache ready for {hash}"),
                Err(e) => log::warn!("Remux cache fill failed for {hash}: {e}"),
            }
        });
        return;
    }

    // Only start on a free slot: a background fill must never become the job
    // the next user-facing conversion waits behind.
    if crate::video_processor::transcode_semaphore().available_permits() == 0 {
        return;
    }
    let output = get_transcoded_path_versioned(
        Path::new(&cache_dir),
        &photo.hash_sha256,
        photo.file_size,
        photo.date_modified.timestamp_millis(),
    );
    if output.exists() {
        // Converted already (or converted while this playthrough ran).
        return;
    }
    if claim_transcode(&photo.hash_sha256) != TranscodeClaim::Started {
        return;
    }
    let conversion = match mode {
        StreamMode::Audio => FileConversion::VideoCopy,
        // `StreamRemux` returned above and `Direct` never streams, so the only
        // remaining mode is a full transcode.
        StreamMode::Transcode | StreamMode::Remux => FileConversion::Reencode,
    };
    spawn_whole_file_transcode(photo, output, conversion, audio_codec);
}

/// Faststart remux sidecar path under `{TRANSCODE_CACHE_DIR}/remux/`, versioned
/// by the source's content fingerprint (size + mtime millis) exactly like the
/// transcode cache, so an in-place edit produces a new sidecar instead of
/// serving a stale moov-at-start copy.
fn remux_sidecar_path(
    cache_dir: &str,
    original_hash: &str,
    file_size: i64,
    modified_millis: i64,
) -> std::path::PathBuf {
    Path::new(cache_dir).join("remux").join(format!(
        "{}_{}_{}.mp4",
        original_hash, file_size, modified_millis
    ))
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
    let status = match get_transcode_status(&photo_hash) {
        Some(status) => status,
        None => return Err(reject::custom(NotFoundError)),
    };

    // Compute how much longer the server will keep a transcode running before
    // giving up, so the polling client can align its timeout with the server's
    // instead of inventing one. Absent when the deadline is unknowable.
    let deadline_ms = status.started_at.map(|started| {
        let timeout_secs = crate::video_processor::transcode_timeout_secs();
        let elapsed_ms = (Utc::now() - started).num_milliseconds().max(0) as u64;
        timeout_secs.saturating_mul(1000).saturating_sub(elapsed_ms)
    });

    let mut body = serde_json::to_value(&status).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(value) = deadline_ms {
        if body.is_object() {
            body.as_object_mut()
                .expect("is_object checked above")
                .insert("deadline_ms".to_string(), serde_json::json!(value));
        }
    }

    Ok(warp::reply::json(&body))
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    /// Seconds into the source; the client restarts the stream here on seek.
    pub start: Option<f64>,
    /// Requested mode. Task 3 derives it from the decision; Task 6 lets the
    /// client escalate only.
    pub mode: Option<String>,
    /// Client codec declaration, same tokens as `?client=` on the decision.
    pub client: Option<String>,
}

/// Upper bound for `?start=` so a hostile client cannot ask for an absurd
/// offset; sources longer than a day are out of scope.
const MAX_STREAM_START_SECS: f64 = 86_400.0;

/// Stream a video as fragmented MP4 straight out of ffmpeg.
///
/// `mode` is an explicit, validated query param here; Task 6 derives it from
/// the playback decision instead, which is why mode selection stays confined
/// to [`StreamMode::from_query`] and the `mode` binding below.
pub async fn stream_video(
    photo_hash: String,
    query: StreamQuery,
    _headers: HeaderMap,
    db_pool: DbPool,
) -> Result<Box<dyn Reply>, Rejection> {
    let photo = match Photo::find_by_hash(&db_pool, &photo_hash).await {
        Ok(Some(photo)) => photo,
        Ok(None) => return Err(reject::custom(NotFoundError)),
        Err(e) => {
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {e}"),
            }))
        }
    };

    let source = Path::new(&photo.file_path);
    let source_size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
    if source_size == 0 {
        let response = warp::reply::with_status(Vec::<u8>::new(), StatusCode::OK);
        let response = warp::reply::with_header(response, "content-length", "0");
        let response = warp::reply::with_header(response, "x-transcode-warning", "empty");
        return Ok(Box::new(response));
    }

    let requested = query.mode.as_deref().unwrap_or("transcode");
    let Some(mode) = StreamMode::from_query(requested) else {
        return Err(reject::custom(NotFoundError));
    };
    let start = query.start.unwrap_or(0.0).clamp(0.0, MAX_STREAM_START_SECS);

    let handle = match start_stream(mode, source, start).await {
        Ok(handle) => handle,
        Err(StreamStartError::Busy) => {
            let response = warp::reply::with_status(
                warp::reply::json(&json!({ "error": "no conversion slot available" })),
                StatusCode::SERVICE_UNAVAILABLE,
            );
            return Ok(Box::new(warp::reply::with_header(
                response,
                "retry-after",
                "2",
            )));
        }
        Err(StreamStartError::Disabled) => {
            let response = warp::reply::with_status(
                warp::reply::json(&json!({ "error": "conversion disabled" })),
                StatusCode::SERVICE_UNAVAILABLE,
            );
            return Ok(Box::new(warp::reply::with_header(
                response,
                "retry-after",
                "5",
            )));
        }
        Err(StreamStartError::Spawn(message)) => {
            log::error!("Stream spawn failed: {message}");
            return Ok(Box::new(warp::reply::with_status(
                warp::reply::json(&json!({ "error": message })),
                StatusCode::INTERNAL_SERVER_ERROR,
            )));
        }
    };

    let mode = handle.mode;
    // Derive the emitted codecs from the resolved capabilities, not the stored
    // record: a first hit on a legacy row probes and persists them, so the
    // MIME the client's SourceBuffer is created with always matches the codecs
    // this run actually copies.
    let caps = crate::video_probe::resolve(&db_pool, &photo).await;
    let duration = caps.duration_secs;

    let StreamHandle {
        stdout,
        stderr,
        child,
        permit,
        ..
    } = handle;
    let hash = photo.hash_sha256.clone();
    // A run that starts at the head of the source is a full playthrough, so a
    // clean finish means the whole file is known-convertible and the cache may
    // be filled; a seek run (`start > 0`) converts only the tail and says
    // nothing about the rest. Either way this happens after the stream the
    // client is already playing, so it can never gate first playback (FR-010).
    let fill_source =
        (start <= FULL_RUN_MAX_START_SECS).then(|| (photo.clone(), caps.audio_codec.clone()));
    tokio::spawn(async move {
        let outcome = supervise(child, stderr).await;
        match &outcome {
            Ok(()) => log::debug!("Stream finished for {hash}"),
            Err(reason) => log::warn!("Stream failed for {hash} ({}): {reason}", mode.as_str()),
        }
        // The stream's own slot is released before the fill asks for one of
        // its own: a full run that sat on a permit while the fill waited would
        // halve the pool for every other conversion.
        drop(permit);
        if outcome.is_ok() {
            if let Some((photo, audio_codec)) = fill_source {
                spawn_cache_fill(&photo, mode, audio_codec.as_deref());
            }
        }
    });

    let body = tokio_util::io::ReaderStream::new(stdout);
    let response = warp::reply::stream(body);
    let response = warp::reply::with_status(response, StatusCode::OK);
    let response = warp::reply::with_header(response, "content-type", "video/mp4");
    let response = warp::reply::with_header(response, "cache-control", "no-store");
    let response = warp::reply::with_header(response, "x-turbopix-mode", mode.as_str());
    let response = warp::reply::with_header(
        response,
        "x-turbopix-mime",
        output_mime(mode, &caps.codec, caps.audio_codec.as_deref()),
    );
    let response: Box<dyn Reply> = match duration {
        Some(secs) => Box::new(warp::reply::with_header(
            response,
            "x-turbopix-duration",
            format!("{secs:.3}"),
        )),
        None => Box::new(response),
    };
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_in_memory_pool;
    use crate::video_processor::clear_transcode_status;
    use crate::video_processor::tests::{acquire_test_env_lock, TestEnvLock};
    use chrono::Utc;
    use tempfile::TempDir;
    use warp::http::HeaderValue;

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
        _lock: TestEnvLock,
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
    async fn empty_backing_file_returns_empty_warning_without_claiming_transcode() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        // setup writes non-empty content; truncate to 0 to simulate a .pending-* file
        let video_path = setup_test_video(&db_pool, &temp_dir, hash).await;
        std::fs::write(&video_path, b"").expect("failed to truncate video");

        // ffprobe path must exist for any probing; is_hevc_video is not reached for the
        // empty branch, but set it defensively to avoid env surprises.
        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'h264\\n'\n");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=0-100"));
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: None,
                decision: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("handler should return")
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("content-length").unwrap(), "0");
        assert_eq!(
            response.headers().get("x-transcode-warning").unwrap(),
            "empty"
        );
        assert!(
            get_transcode_status(hash).is_none(),
            "no transcode may be claimed for an empty file"
        );
    }

    #[tokio::test]
    async fn h264_original_served_directly_without_transcode_param() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let video_path = setup_test_video(&db_pool, &temp_dir, hash).await;
        // Give the photo row a valid h264 codec record so decide() Direct-Plays it.
        use crate::db::Photo;
        let mut photo = Photo::find_by_hash(&db_pool, hash).await.unwrap().unwrap();
        photo.metadata =
            json!({ "video": { "codec": "h264", "container": "mp4", "moov_at_start": true } });
        photo.create_or_update(&db_pool).await.unwrap(); // upsert

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'h264\\n'\n");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=0-10"));
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: None,
                decision: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("handler should return")
        .into_response();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let cr = response
            .headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            cr.ends_with(&format!(
                "/{}",
                std::fs::metadata(&video_path).unwrap().len()
            )),
            "Direct Play must serve the ORIGINAL file bytes"
        );
        assert!(response.headers().get("x-transcode-warning").is_none());
    }

    /// Write a complete capability record (Task 1's `record_is_complete`
    /// contract) so the handler decides without probing the filesystem.
    async fn set_video_record(db_pool: &DbPool, hash: &str, video: serde_json::Value) {
        let patch = json!({ "video": video });
        Photo::persist_metadata_patch(db_pool, hash, &patch)
            .await
            .expect("capability patch");
    }

    /// Run one `?decision` probe for `client` and return the parsed JSON.
    async fn decision_for(db_pool: &DbPool, hash: &str, client: &str) -> serde_json::Value {
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: Some(client.to_string()),
                decision: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool.clone(),
        )
        .await
        .expect("decision handler should return")
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = collect_response_body(response).await;
        serde_json::from_slice(&body).expect("decision response should be JSON")
    }

    #[tokio::test]
    async fn decision_endpoint_reports_direct_and_stream_actions() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "1111111111111111111111111111111111111111111111111111111111111111";
        setup_test_video_with_content(&db_pool, &temp_dir, hash, b"fake-video-data").await;

        // A fake ffprobe that reports an mp4/h264 source: the incomplete-record
        // case below must derive those facts and still decide `direct`.
        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(
            &ffprobe_script,
            "#!/usr/bin/env sh\nprintf '%s' '{\"format\":{\"format_name\":\"mov,mp4,m4a,3gp,3g2,mj2\"},\"streams\":[{\"codec_type\":\"video\",\"codec_name\":\"h264\",\"pix_fmt\":\"yuv420p\",\"disposition\":{\"attached_pic\":0}}]}'\n",
        );
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());

        // Directly playable record: h264 8-bit, mp4, moov at start, aac audio.
        // The decision URL carries the client declaration so the media request
        // re-decides identically.
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "h264", "container": "mp4", "bit_depth": 8,
                "audio_codec": "aac", "moov_at_start": true, "capability_version": 1
            }),
        )
        .await;
        let decision = decision_for(&db_pool, hash, "h264-8,aac").await;
        assert_eq!(decision["action"], "direct");
        assert_eq!(
            decision["url"],
            format!("/api/photos/{hash}/video?client=h264-8%2Caac")
        );
        assert_eq!(decision["mode"], serde_json::Value::Null);
        assert_eq!(decision["mime"], serde_json::Value::Null);
        assert_eq!(decision["duration"], 1.0);
        assert_eq!(decision["cached"], false);

        // Record without `capability_version`: the handler probes the file,
        // persists what it learns, and decides from the DERIVED facts — a
        // legacy row must not be forced into conversion.
        let probed_hash = "1212121212121212121212121212121212121212121212121212121212121212";
        // A separate directory: `setup_test_video_with_content` always writes
        // `video.mp4`, and `photos.file_path` is unique.
        let probed_dir = TempDir::new().expect("failed to create temp dir");
        setup_test_video_with_content(&db_pool, &probed_dir, probed_hash, b"fake-video-data").await;
        {
            let mut photo = Photo::find_by_hash(&db_pool, probed_hash)
                .await
                .unwrap()
                .unwrap();
            photo.metadata = json!({
                "video": { "codec": "h264", "container": "mp4", "moov_at_start": true }
            });
            photo.create_or_update(&db_pool).await.unwrap();
        }
        let decision = decision_for(&db_pool, probed_hash, "h264-8,hevc").await;
        assert_eq!(decision["action"], "direct");
        assert_eq!(
            decision["url"],
            format!("/api/photos/{probed_hash}/video?client=h264-8%2Chevc")
        );
        let probed = Photo::find_by_hash(&db_pool, probed_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(probed.video_codec(), Some("h264"));
        assert_eq!(probed.bit_depth(), Some(8));
        assert_eq!(probed.audio_codec(), None, "no audio stream in the source");

        // Legacy video codec in an AVI container: never natively playable, so
        // the decision streams a full conversion.
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "mpeg4", "container": "avi", "bit_depth": 8,
                "audio_codec": "mp3", "moov_at_start": true, "capability_version": 1
            }),
        )
        .await;
        let decision = decision_for(&db_pool, hash, "h264-8").await;
        assert_eq!(decision["action"], "stream");
        assert_eq!(decision["mode"], "transcode");
        assert_eq!(
            decision["mime"],
            "video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\""
        );
        assert_eq!(
            decision["url"],
            format!("/api/photos/{hash}/video/stream?client=h264-8")
        );

        // HEVC: direct only when the client declares hevc, converted otherwise.
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "hevc", "container": "mp4", "bit_depth": 8,
                "audio_codec": "aac", "moov_at_start": true, "capability_version": 1
            }),
        )
        .await;
        let decision = decision_for(&db_pool, hash, "hevc,aac").await;
        assert_eq!(decision["action"], "direct");
        let decision = decision_for(&db_pool, hash, "h264-8,aac").await;
        assert_eq!(decision["action"], "stream");
        assert_eq!(decision["mode"], "transcode");
        assert_eq!(
            decision["mime"],
            "video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\""
        );

        // moov at the end of an MP4: a lossless remux, never a re-encode.
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "h264", "container": "mp4", "bit_depth": 8,
                "audio_codec": "aac", "moov_at_start": false, "capability_version": 1
            }),
        )
        .await;
        let decision = decision_for(&db_pool, hash, "h264-8,aac").await;
        assert_eq!(decision["action"], "stream");
        assert_eq!(decision["mode"], "remux");
        assert_eq!(
            decision["mime"],
            "video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\""
        );

        // Declared video with undeclared audio: only the audio track converts.
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "h264", "container": "mp4", "bit_depth": 8,
                "audio_codec": "ac3", "moov_at_start": true, "capability_version": 1
            }),
        )
        .await;
        let decision = decision_for(&db_pool, hash, "h264-8,aac").await;
        assert_eq!(decision["action"], "stream");
        assert_eq!(decision["mode"], "audio");
        assert_eq!(
            decision["mime"],
            "video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\""
        );
    }

    #[tokio::test]
    async fn decision_serves_a_completed_cached_transcode() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "0909090909090909090909090909090909090909090909090909090909090909";
        clear_transcode_status(hash);
        let _video_path = setup_test_video(&db_pool, &temp_dir, hash).await;
        // A record this client cannot play: mpeg4 in an AVI always converts.
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "mpeg4", "container": "avi", "bit_depth": 8,
                "audio_codec": "mp3", "moov_at_start": true, "capability_version": 1
            }),
        )
        .await;

        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        // Nothing cached yet: the decision streams, it never waits on a
        // conversion (FR-010).
        let decision = decision_for(&db_pool, hash, "h264-8").await;
        assert_eq!(decision["action"], "stream");
        assert_eq!(decision["mode"], "transcode");
        assert_eq!(decision["cached"], false);

        // Cache a conversion under the versioned name (size + mtime are folded
        // in, so the handler's lookup and this write must agree)…
        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let cached = get_transcoded_path_versioned(
            temp_dir.path(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        std::fs::create_dir_all(cached.parent().unwrap()).expect("failed to create cache dir");
        std::fs::write(&cached, b"cached-transcode").expect("failed to write cached transcode");

        // …and the decision plays it as a file instead of converting again.
        let decision = decision_for(&db_pool, hash, "h264-8").await;
        assert_eq!(decision["action"], "direct");
        assert_eq!(decision["cached"], true);
        assert_eq!(
            decision["url"],
            format!("/api/photos/{hash}/video?client=h264-8&transcode=true")
        );

        // A file left behind by a failed/timed-out attempt is not an artifact:
        // the serve path deletes it and falls back to the original, so the
        // decision must not advertise it as cached either.
        set_transcode_status(
            hash,
            TranscodeStatus {
                state: TranscodeState::Failed,
                hash: hash.to_string(),
                started_at: Some(Utc::now()),
                error: Some("boom".to_string()),
                percent: None,
            },
        );
        let decision = decision_for(&db_pool, hash, "h264-8").await;
        assert_eq!(decision["action"], "stream");
        assert_eq!(decision["cached"], false);
        clear_transcode_status(hash);
    }

    #[tokio::test]
    async fn cached_remux_sidecar_is_played_instead_of_streamed() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b";
        clear_transcode_status(hash);
        setup_test_video_with_content(&db_pool, &temp_dir, hash, b"original-bytes").await;
        // h264 + aac that only needs its moov moved: a remux, never a re-encode.
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "h264", "container": "mp4", "bit_depth": 8,
                "audio_codec": "aac", "moov_at_start": false, "capability_version": 1
            }),
        )
        .await;

        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let decision = decision_for(&db_pool, hash, "h264-8,aac").await;
        assert_eq!(decision["action"], "stream");
        assert_eq!(decision["mode"], "remux");
        assert_eq!(decision["cached"], false);

        // Once the sidecar exists the remux run has nothing left to produce.
        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let sidecar = remux_sidecar_path(
            temp_dir.path().to_str().unwrap(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        std::fs::create_dir_all(sidecar.parent().unwrap()).expect("failed to create cache dir");
        std::fs::write(&sidecar, b"faststart-sidecar").expect("failed to write sidecar");

        let decision = decision_for(&db_pool, hash, "h264-8,aac").await;
        assert_eq!(decision["action"], "direct");
        assert_eq!(decision["cached"], true);
        let url = decision["url"].as_str().expect("url is a string");
        assert_eq!(url, format!("/api/photos/{hash}/video?client=h264-8%2Caac"));

        // …and that URL serves the sidecar bytes, not the original.
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: Some("h264-8,aac".to_string()),
                decision: None,
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("byte request should be served")
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = collect_response_body(response).await;
        assert_eq!(body, b"faststart-sidecar");
    }

    #[tokio::test]
    async fn remux_short_circuits_on_already_progressive_serves_original() {
        // A photo whose record says moov_at_start: false but whose backing file
        // is already progressive (index-time moov fix failed, then the file was
        // fixed/replaced on disk): ensure_progressive_mp4 no-ops without writing
        // the sidecar, so the handler must serve the playable ORIGINAL instead
        // of a nonexistent remux path (which would 404).
        let fixture = Path::new("test-data/test_video.mp4");
        if !fixture.exists() {
            eprintln!("skipping: test_video.mp4 fixture missing");
            return;
        }

        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "f3".repeat(32);

        let _ = setup_test_video(&db_pool, &temp_dir, &hash).await;
        // Point the photo at the real progressive fixture and record moov-at-end.
        let real_size = std::fs::metadata(fixture).unwrap().len();
        use crate::db::Photo;
        let mut photo = Photo::find_by_hash(&db_pool, &hash).await.unwrap().unwrap();
        photo.file_path = fixture.to_str().unwrap().to_string();
        photo.filename = "test_video.mp4".to_string();
        photo.file_size = real_size as i64;
        photo.metadata = json!({
            "video": { "codec": "h264", "container": "mp4", "moov_at_start": false }
        });
        photo.create_or_update(&db_pool).await.unwrap();

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(&ffprobe_script, "#!/usr/bin/env sh\nprintf 'h264\\n'\n");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=0-1023"));
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: None,
                decision: None,
            },
            headers,
            db_pool,
        )
        .await
        .expect("handler should return")
        .into_response();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let cr = response
            .headers()
            .get("content-range")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            cr.ends_with(&format!("/{}", real_size)),
            "must serve the ORIGINAL progressive file bytes, got content-range {}",
            cr
        );
        // The short-circuit must NOT have left a sidecar behind.
        let remux_path = remux_sidecar_path(
            temp_dir.path().to_str().unwrap(),
            &hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        assert!(
            !remux_path.exists(),
            "no remux sidecar should be written when the source is already progressive"
        );
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
                client_codecs: None,
                decision: None,
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
            percent: None,
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
    async fn test_video_status_includes_percent_and_deadline() {
        let hash = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        clear_transcode_status(hash);

        let started_at = Utc::now();
        set_transcode_status(
            hash,
            TranscodeStatus {
                state: TranscodeState::InProgress,
                hash: hash.to_string(),
                started_at: Some(started_at),
                error: None,
                percent: Some(42),
            },
        );

        let response = get_video_status(hash.to_string())
            .await
            .expect("status should return");
        let body = collect_response_body(response.into_response()).await;
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("status response should be JSON");

        assert_eq!(json["percent"], 42, "percent must be serialized");
        let deadline_ms = json["deadline_ms"]
            .as_u64()
            .expect("deadline_ms must be present");
        // Default timeout 300s, minus the (tiny) elapsed time since started_at.
        assert!(
            deadline_ms > 250_000 && deadline_ms <= 300_000,
            "deadline_ms should be ~300s minus elapsed, got {}",
            deadline_ms
        );

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
                percent: None,
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
                percent: None,
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

        // Cache the transcode under the VERSIONED name (size + mtime are
        // folded in, so the handler's lookup and this write must agree).
        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let transcoded_path = get_transcoded_path_versioned(
            temp_dir.path(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        std::fs::create_dir_all(transcoded_path.parent().unwrap())
            .expect("failed to create cache dir");
        std::fs::write(&transcoded_path, b"cached-transcoded-video")
            .expect("failed to write cached transcoded video");

        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
                client_codecs: None,
                decision: None,
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

    /// Poll the transcode status until the background fill reports Completed;
    /// `spawn_cache_fill` spawns its work, so the test cannot await the task.
    async fn wait_for_completed_transcode(hash: &str) {
        for _ in 0..200 {
            if matches!(
                get_transcode_status(hash).map(|status| status.state),
                Some(TranscodeState::Completed)
            ) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("transcode for {hash} never completed");
    }

    #[tokio::test]
    async fn cache_fill_converts_the_source_after_a_playthrough() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c";
        clear_transcode_status(hash);
        let _video_path = setup_test_video(&db_pool, &temp_dir, hash).await;

        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(
            &ffprobe_script,
            "#!/usr/bin/env sh\nprintf '{\"format\":{\"duration\":\"1.0\"}}'\n",
        );
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg.sh");
        // The conversion writes its output file (the transcode renames it into
        // the cache itself).
        create_script(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\nprintf 'converted' > \"$last\"\n",
        );

        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let cached = get_transcoded_path_versioned(
            temp_dir.path(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        assert!(!cached.exists(), "nothing is cached before the fill");

        spawn_cache_fill(&photo, StreamMode::Transcode, Some("mp3"));
        wait_for_completed_transcode(hash).await;

        assert_eq!(
            std::fs::read_to_string(&cached).expect("fill must land in the cache"),
            "converted"
        );
    }

    #[tokio::test]
    async fn cache_fill_leaves_converted_and_claimed_hashes_alone() {
        let db_pool = create_in_memory_pool().await.expect("failed to create db");
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let hash = "0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d0d";
        clear_transcode_status(hash);
        let _video_path = setup_test_video(&db_pool, &temp_dir, hash).await;

        // An encoder that would overwrite the artifact if it ever ran.
        let ffmpeg_script = temp_dir.path().join("second_ffmpeg.sh");
        create_script(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\nprintf 'second-run' > \"$last\"\n",
        );
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let cached = get_transcoded_path_versioned(
            temp_dir.path(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        std::fs::create_dir_all(cached.parent().unwrap()).expect("failed to create cache dir");
        std::fs::write(&cached, b"converted").expect("failed to write cached transcode");

        // Already converted: the artifact is the whole point, so a second
        // playthrough must not re-encode it.
        spawn_cache_fill(&photo, StreamMode::Transcode, Some("mp3"));

        // Already converting (another request owns the claim): the pool bound
        // admits one encoder per hash, so this playthrough must not queue a
        // second one either.
        std::fs::remove_file(&cached).expect("failed to clear the artifact");
        assert_eq!(claim_transcode(hash), TranscodeClaim::Started);
        spawn_cache_fill(&photo, StreamMode::Transcode, Some("mp3"));

        // Neither call may have started an encoder.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            !cached.exists(),
            "no fill may run for an already converted or already claimed hash"
        );
        clear_transcode_status(hash);
    }

    /// A run the client could decode as-is (audio conversion only) must cache
    /// the same thing it streamed: the video track is copied, so the artifact
    /// the next open is served is not a re-encode of video that already played.
    #[tokio::test]
    async fn audio_playthrough_fills_a_video_copy_artifact() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";
        clear_transcode_status(hash);
        setup_test_video(&db_pool, &temp_dir, hash).await;
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "h264", "container": "mp4", "bit_depth": 8,
                "audio_codec": "ac3", "moov_at_start": true, "capability_version": 1
            }),
        )
        .await;

        let args_file = temp_dir.path().join("convert-args.txt");
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg.sh");
        create_script(
            &ffmpeg_script,
            &format!(
                "#!/usr/bin/env sh\nlast=\"\"\nfor arg in \"$@\"; do last=\"$arg\"; done\nif [ \"$last\" = \"pipe:1\" ]; then\n  printf '\\000\\000\\000\\030ftypiso5'\nelse\n  printf '%s\\n' \"$@\" > '{}'\n  printf 'converted' > \"$last\"\nfi\n",
                args_file.display()
            ),
        );
        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(
            &ffprobe_script,
            "#!/usr/bin/env sh\nprintf '{\"format\":{\"duration\":\"1.0\"}}'\n",
        );
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let response = stream_video(
            hash.to_string(),
            StreamQuery {
                start: Some(0.0),
                mode: Some("audio".to_string()),
                client: None,
            },
            HeaderMap::new(),
            db_pool.clone(),
        )
        .await
        .expect("stream should reply")
        .into_response();
        let _ = collect_response_body(response).await;

        wait_for_completed_transcode(hash).await;
        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let cached = get_transcoded_path_versioned(
            temp_dir.path(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        assert!(
            cached.exists(),
            "the playthrough must leave a whole-file artifact"
        );

        let args = std::fs::read_to_string(&args_file).expect("conversion args must be recorded");
        assert!(
            args.lines().any(|line| line == "-c:v") && args.contains("\ncopy\n"),
            "the video track must be copied, not re-encoded: {args}"
        );
        assert!(
            !args.contains("libx264"),
            "an audio-mode cache must not re-encode video: {args}"
        );
        assert!(
            args.contains("-c:a") && args.contains("\naac\n"),
            "the undecodable audio must be converted: {args}"
        );

        // …and the next open is served that artifact instead of streaming.
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: Some("h264-8,aac".to_string()),
                decision: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("decision reply")
        .into_response();
        let body = collect_response_body(response).await;
        let decision: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(decision["action"], "direct");
        assert_eq!(decision["cached"], true);
    }

    /// A remux run is a container/layout copy, so the cache holds exactly that:
    /// the lossless sidecar the decision reuses, never a re-encode of video the
    /// client already plays. The fill remuxes unconditionally — the run that
    /// just finished is the proof it was needed — which is also what makes a
    /// Matroska source (no moov for the serve-time probe to find) cacheable.
    #[tokio::test]
    async fn remux_playthrough_fills_the_lossless_sidecar() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a";
        clear_transcode_status(hash);
        setup_test_video(&db_pool, &temp_dir, hash).await;
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "h264", "container": "mp4", "bit_depth": 8,
                "audio_codec": "aac", "moov_at_start": false, "capability_version": 1
            }),
        )
        .await;

        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg.sh");
        create_script(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\nif [ \"$last\" = \"pipe:1\" ]; then\n  printf '\\000\\000\\000\\030ftypiso5'\nelse\n  printf 'faststart-sidecar' > \"$last\"\nfi\n",
        );
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let response = stream_video(
            hash.to_string(),
            StreamQuery {
                start: Some(0.0),
                mode: Some("remux".to_string()),
                client: None,
            },
            HeaderMap::new(),
            db_pool.clone(),
        )
        .await
        .expect("stream should reply")
        .into_response();
        let _ = collect_response_body(response).await;

        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let sidecar = remux_sidecar_path(
            temp_dir.path().to_str().unwrap(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        for _ in 0..200 {
            if sidecar.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            std::fs::read_to_string(&sidecar).expect("the playthrough must fill the sidecar"),
            "faststart-sidecar"
        );
        let transcoded = get_transcoded_path_versioned(
            temp_dir.path(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        assert!(
            !transcoded.exists(),
            "a remux run must not leave a re-encoded artifact behind"
        );

        // The sidecar is exactly what the decision's cached arm looks for.
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: Some("h264-8,aac".to_string()),
                decision: Some("true".to_string()),
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("decision reply")
        .into_response();
        let body = collect_response_body(response).await;
        let decision: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(decision["action"], "direct");
        assert_eq!(decision["cached"], true);
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
                client_codecs: None,
                decision: None,
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

        // A range request against an empty file now hits the empty-file
        // fast-fail BEFORE range processing: 200 with content-length 0 and an
        // "empty" warning (the file can never be played, remuxed, or
        // transcoded, so no transcode slot is claimed).
        let mut headers = HeaderMap::new();
        headers.insert("range", HeaderValue::from_static("bytes=0-"));
        let response = get_video_file(
            hash.to_string(),
            VideoQuery {
                metadata: None,
                transcode: None,
                client_codecs: None,
                decision: None,
            },
            headers,
            db_pool,
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
        assert_eq!(
            response
                .headers()
                .get("x-transcode-warning")
                .and_then(|v| v.to_str().ok()),
            Some("empty")
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
                client_codecs: None,
                decision: None,
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
                client_codecs: None,
                decision: None,
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
                client_codecs: None,
                decision: None,
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
                client_codecs: None,
                decision: None,
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
                client_codecs: None,
                decision: None,
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
                client_codecs: None,
                decision: None,
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

        // A previous transcode attempt failed, leaving a corrupt file behind
        // (under the VERSIONED name the handler looks up).
        let photo = Photo::find_by_hash(&db_pool, &hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let transcoded_path = get_transcoded_path_versioned(
            temp_dir.path(),
            &hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        std::fs::create_dir_all(transcoded_path.parent().unwrap())
            .expect("failed to create cache dir");
        std::fs::write(&transcoded_path, b"partial-corrupt-output")
            .expect("failed to write stale transcoded video");
        set_transcode_status(
            &hash,
            TranscodeStatus {
                state: TranscodeState::Failed,
                hash: hash.clone(),
                started_at: Some(Utc::now()),
                error: Some("ffmpeg transcode exited with status 1".to_string()),
                percent: None,
            },
        );

        let response = get_video_file(
            hash.clone(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
                client_codecs: None,
                decision: None,
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
                client_codecs: None,
                decision: None,
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
        // no file exists at the cache path -- only a leftover temp sibling
        // (under the VERSIONED name the handler looks up).
        let photo = Photo::find_by_hash(&db_pool, &hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let transcoded_path = get_transcoded_path_versioned(
            temp_dir.path(),
            &hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
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
                started_at: Some(Utc::now()),
                error: Some("ffmpeg transcode exited with status 1".to_string()),
                percent: None,
            },
        );

        let response = get_video_file(
            hash.clone(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
                client_codecs: None,
                decision: None,
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

        let video_path = setup_test_video(&db_pool, &temp_dir, &hash).await;

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
                percent: None,
            },
        );

        let response = get_video_file(
            hash.clone(),
            VideoQuery {
                metadata: None,
                transcode: Some("true".to_string()),
                client_codecs: None,
                decision: None,
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
        // Version args come from the REAL source file: the handler's lookup
        // path folds in size+mtime, so asserting the matching path is
        // meaningful (any file there would be the version the handler serves).
        let meta = std::fs::metadata(&video_path).expect("source video should exist");
        let mtime_ms = meta
            .modified()
            .expect("mtime")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("post-epoch")
            .as_millis() as i64;
        assert!(
            !get_transcoded_path_versioned(temp_dir.path(), &hash, meta.len() as i64, mtime_ms)
                .exists(),
            "no transcode output should exist"
        );

        clear_transcode_status(&hash);
    }

    /// Take every conversion permit and keep holding them, so the transcode pool
    /// stays saturated for as long as the returned guards live. Drain, pause,
    /// drain again: another test's ffmpeg can still be running inside the
    /// process-wide semaphore and release a permit after the first drain, which
    /// would leave the requests under test with a slot to grab.
    async fn saturate_transcode_pool() -> Vec<tokio::sync::SemaphorePermit<'static>> {
        let semaphore = crate::video_processor::transcode_semaphore();
        for _ in 0..40 {
            let mut held = Vec::new();
            while let Ok(permit) = semaphore.try_acquire() {
                held.push(permit);
            }
            assert!(!held.is_empty(), "at least one permit must be acquirable");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if semaphore.available_permits() == 0 {
                return held;
            }
        }
        panic!("the transcode pool never settled into a saturated state");
    }

    #[tokio::test]
    async fn stream_endpoint_serves_fragmented_bytes_with_mode_header() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        setup_test_video(&db_pool, &temp_dir, hash).await;

        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg.sh");
        create_script(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nprintf '\\000\\000\\000\\030ftypiso5'\nprintf '\\000\\000\\000\\010moov'\n",
        );
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let response = stream_video(
            hash.to_string(),
            StreamQuery {
                start: Some(0.0),
                mode: Some("remux".to_string()),
                client: None,
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("stream should reply");
        let response = response.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-turbopix-mode"], "remux");
        assert_eq!(response.headers()["content-type"], "video/mp4");

        let body = collect_response_body(response).await;
        assert!(body.windows(4).any(|w| w == b"ftyp"));
    }

    #[tokio::test]
    async fn full_stream_fills_the_cache_but_a_seek_run_does_not() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e";
        clear_transcode_status(hash);
        setup_test_video(&db_pool, &temp_dir, hash).await;
        set_video_record(
            &db_pool,
            hash,
            json!({
                "codec": "mpeg4", "container": "avi", "bit_depth": 8,
                "audio_codec": "ac3", "moov_at_start": true, "capability_version": 1
            }),
        )
        .await;

        // One fake encoder for both runs: fragmented MP4 on stdout for the
        // stream, the output file for the whole-file conversion.
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg.sh");
        create_script(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\nif [ \"$last\" = \"pipe:1\" ]; then\n  printf '\\000\\000\\000\\030ftypiso5'\nelse\n  printf 'converted' > \"$last\"\nfi\n",
        );
        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        create_script(
            &ffprobe_script,
            "#!/usr/bin/env sh\nprintf '{\"format\":{\"duration\":\"1.0\"}}'\n",
        );
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _cache_guard =
            EnvVarGuard::set("TRANSCODE_CACHE_DIR", temp_dir.path().to_str().unwrap());

        let photo = Photo::find_by_hash(&db_pool, hash)
            .await
            .expect("find failed")
            .expect("photo should exist");
        let cached = get_transcoded_path_versioned(
            temp_dir.path(),
            hash,
            photo.file_size,
            photo.date_modified.timestamp_millis(),
        );
        assert!(!cached.exists());

        // A run from the head of the source is a full playthrough: once it ends
        // (the body reaches EOF) the whole-file conversion runs in the
        // background, bounded by the same claim and pool as every other
        // conversion.
        let response = stream_video(
            hash.to_string(),
            StreamQuery {
                start: Some(0.0),
                mode: Some("transcode".to_string()),
                client: None,
            },
            HeaderMap::new(),
            db_pool.clone(),
        )
        .await
        .expect("stream should reply")
        .into_response();
        let body = collect_response_body(response).await;
        assert!(body.windows(4).any(|w| w == b"ftyp"));

        wait_for_completed_transcode(hash).await;
        assert_eq!(
            std::fs::read_to_string(&cached).expect("the playthrough must fill the cache"),
            "converted"
        );

        // A seek run converts only the tail of the source, so it must not claim
        // a whole-file artifact for a file nobody has watched from the start.
        std::fs::remove_file(&cached).expect("failed to clear the artifact");
        clear_transcode_status(hash);
        let response = stream_video(
            hash.to_string(),
            StreamQuery {
                start: Some(15.0),
                mode: Some("transcode".to_string()),
                client: None,
            },
            HeaderMap::new(),
            db_pool.clone(),
        )
        .await
        .expect("stream should reply")
        .into_response();
        let _ = collect_response_body(response).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            !cached.exists(),
            "a seek run must not fill the whole-file cache"
        );
    }

    #[tokio::test]
    async fn stream_endpoint_returns_503_when_pool_is_saturated() {
        let db_pool = create_in_memory_pool().await.expect("db");
        let temp_dir = TempDir::new().unwrap();
        let hash = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        setup_test_video(&db_pool, &temp_dir, hash).await;

        let _wait_guard = EnvVarGuard::set("TURBO_PIX_STREAM_QUEUE_WAIT_SECS", "0");

        // Hold every permit so the request cannot be served. This must not rely
        // on TURBO_PIX_MAX_TRANSCODES: the semaphore is a OnceLock sized by the
        // first caller in the process.
        let held = saturate_transcode_pool().await;

        let response = stream_video(
            hash.to_string(),
            StreamQuery {
                start: Some(0.0),
                mode: Some("transcode".to_string()),
                client: None,
            },
            HeaderMap::new(),
            db_pool,
        )
        .await
        .expect("busy reply");
        let response = response.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "2");

        drop(held);
    }
}
