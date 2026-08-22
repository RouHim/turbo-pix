use crate::thumbnail_types::{CacheError, CacheResult, VideoMetadata};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::timeout;

// Transcoding status tracking types and in-memory store
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum TranscodeState {
    Pending,
    InProgress,
    Completed,
    Failed,
    Timeout,
}

#[derive(Serialize, Clone, Debug)]
pub struct TranscodeStatus {
    pub state: TranscodeState,
    pub hash: String,
    pub started_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// Progress percentage of the current transcode (0..=100), when known.
    /// `None` when no progress signal is available (e.g. duration unknown).
    pub percent: Option<u8>,
}

static TRANSCODE_STATUS_STORE: OnceLock<Mutex<HashMap<String, TranscodeStatus>>> = OnceLock::new();
static TRANSCODE_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

/// Maximum number of transcode status entries kept in memory. The store is a
/// status cache for polling clients; settled entries are evicted first when
/// the cap is exceeded so the map cannot grow without bound. Under a burst of
/// concurrent transcodes the cap acts as a soft limit: in-progress entries are
/// never evicted, since removing them would break in-flight polls.
const TRANSCODE_STATUS_STORE_CAP: usize = 128;

fn get_status_store() -> &'static Mutex<HashMap<String, TranscodeStatus>> {
    TRANSCODE_STATUS_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Bounded worker pool for transcodes. Defaults to `min(max(nproc/2,1),4)`
/// unless `TURBO_PIX_MAX_TRANSCODES` pins it (0 = transcoding disabled). This
/// replaces the historical global `Semaphore::new(1)` so distinct HEVC/non-HEVC
/// re-encodes can run concurrently instead of serializing behind one job.
pub fn transcode_semaphore() -> &'static Semaphore {
    TRANSCODE_SEMAPHORE.get_or_init(|| Semaphore::new(transcode_max_pool()))
}

/// Number of concurrent transcode jobs. Reads env each call so a runtime value
/// is honored; the semaphore itself locks in the first value it saw via
/// [`transcode_semaphore`].
pub fn transcode_max_pool() -> usize {
    match std::env::var("TURBO_PIX_MAX_TRANSCODES") {
        Ok(raw) => raw.trim().parse::<usize>().unwrap_or_else(|_| {
            log::warn!(
                "Invalid TURBO_PIX_MAX_TRANSCODES '{}', using default 2",
                raw
            );
            2
        }),
        Err(_) => {
            std::thread::available_parallelism().map_or(2, |n| (n.get().max(2) / 2).clamp(1, 4))
        }
    }
}

/// Per-transcode timeout in seconds, from `TURBO_PIX_TRANSCODE_TIMEOUT_SECS`
/// (default 300). Exposed to clients via the status endpoint so polling stops
/// when the server would actually give up, not at an arbitrary client cap.
pub fn transcode_timeout_secs() -> u64 {
    match std::env::var("TURBO_PIX_TRANSCODE_TIMEOUT_SECS") {
        Ok(raw) => raw.trim().parse::<u64>().unwrap_or_else(|_| {
            log::warn!(
                "Invalid TURBO_PIX_TRANSCODE_TIMEOUT_SECS '{}', using default 300",
                raw
            );
            300
        }),
        Err(_) => 300,
    }
}

// NOTE: no test-only reset hook is provided for TRANSCODE_SEMAPHORE. The
// semaphore caches its size from the first transcode in the process; tests
// that need pool semantics must use env values that are immune to that
// (transcode_max_pool() is read fresh per call, so env parsing is testable;
// the disabled path checks transcode_max_pool() == 0 before touching the
// semaphore, so it is deterministic regardless of prior initialization).

// Lightweight semaphore for serve-time moov faststart remuxes. Deliberately
// distinct from TRANSCODE_SEMAPHORE: a remux is a fast `-c copy` stream copy
// that must not queue behind slow re-encodes. Bounded to 4 so at most 4
// concurrent ffmpeg remux processes run at once.
static REMOX_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

/// Monotonic counter for giving each remux temp file a unique name (see
/// `remux_temp_path`). The remux path lacks the per-hash claim the transcode
/// path has, so a fixed temp name would let two concurrent requests for the
/// same sidecar write the same file (interleaved writes under `-y`).
static REMUX_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn get_remox_semaphore() -> &'static Semaphore {
    REMOX_SEMAPHORE.get_or_init(|| Semaphore::new(4))
}

pub async fn acquire_transcode_permit() -> CacheResult<SemaphorePermit<'static>> {
    if transcode_max_pool() == 0 {
        return Err(CacheError::VideoProcessingError(
            "Transcoding is disabled (TURBO_PIX_MAX_TRANSCODES=0)".to_string(),
        ));
    }
    transcode_semaphore().acquire().await.map_err(|e| {
        CacheError::VideoProcessingError(format!("Failed to acquire transcode permit: {}", e))
    })
}

/// Evict entries from the status map until it is at or under
/// [`TRANSCODE_STATUS_STORE_CAP`]. Only settled entries (Completed, and
/// Failed/Timeout OLDER than the retry cooldown) are removed; in-progress
/// entries are never evicted so polling clients keep a live status, and
/// fresh failures keep their cooldown (evicting them would let a doomed
/// 300s job re-spawn immediately). If only protected entries remain,
/// eviction stops and the cap acts as a soft limit under bursts of
/// concurrent transcodes.
fn evict_transcode_statuses(map: &mut HashMap<String, TranscodeStatus>) {
    if map.len() <= TRANSCODE_STATUS_STORE_CAP {
        return;
    }

    let settled: Vec<String> = map
        .iter()
        .filter(|(_, s)| match s.state {
            TranscodeState::Completed => true,
            TranscodeState::Failed | TranscodeState::Timeout => s
                .started_at
                .is_none_or(|started| started + TRANSCODE_RETRY_COOLDOWN < Utc::now()),
            TranscodeState::InProgress | TranscodeState::Pending => false,
        })
        .map(|(key, _)| key.clone())
        .collect();
    for key in settled {
        map.remove(&key);
        if map.len() <= TRANSCODE_STATUS_STORE_CAP {
            return;
        }
    }
}

pub fn set_transcode_status(hash: &str, status: TranscodeStatus) {
    let store = get_status_store();
    if let Ok(mut map) = store.lock() {
        map.insert(hash.to_string(), status);
        evict_transcode_statuses(&mut map);
    }
}

pub fn get_transcode_status(hash: &str) -> Option<TranscodeStatus> {
    let store = get_status_store();
    store.lock().ok().and_then(|map| map.get(hash).cloned())
}

/// Outcome of atomically claiming the transcode slot for a hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscodeClaim {
    /// This caller owns the slot and must spawn the transcode job.
    Started,
    /// A transcode for this hash is already running; return the poll response
    /// without starting a second job.
    AlreadyInProgress,
    /// A previous attempt failed or timed out; remove any leftover temp file
    /// and serve the original.
    PreviouslyFailedOrTimedOut,
    /// The worker pool is at its concurrent-claim cap; serve the original
    /// instead of queueing an unbounded spawned task.
    PoolSaturated,
}

/// How long a Failed/Timeout transcode status blocks re-spawning the same
/// hash. Immediately repeating a doomed request should keep serving the
/// original, but a permanently blocked hash would never recover from a
/// transient failure (OOM, codec hiccup, disk-full) without a server restart.
const TRANSCODE_RETRY_COOLDOWN: chrono::Duration = chrono::Duration::minutes(15);

/// Atomically consults and claims the transcode slot for `hash` under the
/// status-store lock, closing the check-then-act window where two concurrent
/// requests for the same hash could both read "no status" and each spawn an
/// ffmpeg job. The global transcode semaphore would only serialize the jobs,
/// not prevent the duplicate spawn. Claims are additionally bounded to the
/// worker-pool size so a flood of distinct-hash transcode requests cannot
/// accumulate unbounded spawned tasks or unbounded InProgress status entries.
pub fn claim_transcode(hash: &str) -> TranscodeClaim {
    let store = get_status_store();
    let mut map = store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // A new claim is allowed only while fewer than `transcode_max_pool()`
    // hashes are already in flight. This mirrors the semaphore's concurrency
    // bound and keeps both the spawned-task count and the InProgress status
    // map bounded. A full pool (including 0 = disabled) serves the original.
    let can_start = count_in_progress(&map) < transcode_max_pool();

    match map.get(hash) {
        Some(status) if matches!(status.state, TranscodeState::InProgress) => {
            TranscodeClaim::AlreadyInProgress
        }
        Some(status)
            if matches!(
                status.state,
                TranscodeState::Failed | TranscodeState::Timeout
            ) =>
        {
            // A failure older than the cooldown is retried (transient
            // failures heal); a fresh one keeps serving the original.
            let stale = status
                .started_at
                .is_none_or(|started| started + TRANSCODE_RETRY_COOLDOWN < Utc::now());
            if stale {
                if !can_start {
                    return TranscodeClaim::PoolSaturated;
                }
                map.insert(hash.to_string(), in_progress_status(hash));
                evict_transcode_statuses(&mut map);
                TranscodeClaim::Started
            } else {
                TranscodeClaim::PreviouslyFailedOrTimedOut
            }
        }
        _ => {
            if !can_start {
                return TranscodeClaim::PoolSaturated;
            }
            map.insert(hash.to_string(), in_progress_status(hash));
            evict_transcode_statuses(&mut map);
            TranscodeClaim::Started
        }
    }
}

fn count_in_progress(map: &HashMap<String, TranscodeStatus>) -> usize {
    map.values()
        .filter(|s| matches!(s.state, TranscodeState::InProgress))
        .count()
}

fn in_progress_status(hash: &str) -> TranscodeStatus {
    TranscodeStatus {
        state: TranscodeState::InProgress,
        hash: hash.to_string(),
        started_at: Some(Utc::now()),
        error: None,
        percent: None,
    }
}

pub fn clear_transcode_status(hash: &str) {
    let store = get_status_store();
    if let Ok(mut map) = store.lock() {
        map.remove(hash);
    }
}

pub fn get_ffmpeg_path() -> String {
    std::env::var("FFMPEG_PATH").unwrap_or_else(|_| "ffmpeg".to_string())
}

pub fn get_ffprobe_path() -> String {
    std::env::var("FFPROBE_PATH").unwrap_or_else(|_| "ffprobe".to_string())
}

pub fn format_binary_error(binary_name: &str, path: &str, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        let env_var = format!("{}_PATH", binary_name.to_uppercase());
        return format!(
            "{binary_name} binary not found at '{path}'. Set {env_var} environment variable to the correct path."
        );
    }

    format!("{binary_name} failed to execute at '{path}': {error}")
}

fn verify_binary_available(binary_name: &str, path: &str) -> Result<(), String> {
    let output = std::process::Command::new(path)
        .arg("-version")
        .output()
        .map_err(|error| format_binary_error(binary_name, path, &error))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        return Err(format!(
            "{binary_name} failed to execute at '{path}': exited with status {}",
            output.status
        ));
    }

    Err(format!(
        "{binary_name} failed to execute at '{path}': {detail}"
    ))
}

pub fn verify_ffmpeg_available() -> Result<(), String> {
    let ffmpeg_path = get_ffmpeg_path();
    verify_binary_available("ffmpeg", &ffmpeg_path)?;

    let ffprobe_path = get_ffprobe_path();
    verify_binary_available("ffprobe", &ffprobe_path)
}

pub async fn extract_video_metadata(video_path: &Path) -> CacheResult<VideoMetadata> {
    let video_path = video_path.to_path_buf();
    let ffprobe_path = get_ffprobe_path();
    let ffprobe_path_for_err = ffprobe_path.clone();

    let output = tokio::task::spawn_blocking(move || {
        Command::new(ffprobe_path)
            .args([
                "-v",
                "quiet",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                video_path.to_string_lossy().as_ref(),
            ])
            .output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| {
        CacheError::VideoProcessingError(format_binary_error("ffprobe", &ffprobe_path_for_err, &e))
    })?;

    if !output.status.success() {
        return Err(CacheError::VideoProcessingError(format!(
            "ffprobe exited with status: {}",
            output.status
        )));
    }

    let json_str = String::from_utf8(output.stdout)
        .map_err(|e| CacheError::VideoProcessingError(format!("Invalid UTF-8 output: {}", e)))?;

    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| CacheError::VideoProcessingError(format!("JSON parse error: {}", e)))?;

    // Extract duration from format section
    let duration = parsed["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| CacheError::VideoMetadataError("Duration not found".to_string()))?;

    // Extract width/height from first video stream
    let streams = parsed["streams"]
        .as_array()
        .ok_or_else(|| CacheError::VideoMetadataError("No streams found".to_string()))?;

    let video_stream = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .ok_or_else(|| CacheError::VideoMetadataError("No video stream found".to_string()))?;

    let width = video_stream["width"]
        .as_i64()
        .ok_or_else(|| CacheError::VideoMetadataError("Width not found".to_string()))?
        as i32;

    let height = video_stream["height"]
        .as_i64()
        .ok_or_else(|| CacheError::VideoMetadataError("Height not found".to_string()))?
        as i32;

    Ok(VideoMetadata {
        duration,
        width,
        height,
    })
}

pub fn calculate_optimal_frame_time(metadata: &VideoMetadata) -> f64 {
    let duration = metadata.duration;

    // Extract frame at 10% of duration, with constraints
    let optimal_time = duration * 0.1;

    // Apply constraints: minimum 0.5s, maximum 30s
    if optimal_time < 0.5 {
        (0.5f64).min(duration * 0.5) // For very short videos, take middle frame
    } else if optimal_time > 30.0 {
        30.0
    } else {
        optimal_time
    }
}

pub async fn extract_frame_at_time(
    video_path: &Path,
    time_seconds: f64,
    output_path: &Path,
) -> CacheResult<()> {
    let video_path = video_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let ffmpeg_path = get_ffmpeg_path();
    let ffmpeg_path_for_err = ffmpeg_path.clone();
    let time_str = time_seconds.to_string();

    let output = tokio::task::spawn_blocking(move || {
        Command::new(ffmpeg_path)
            .args([
                "-y", // Overwrite output file
                "-ss",
                &time_str, // Fast seeking: place BEFORE -i for input-level seek
                "-i",
                video_path.to_string_lossy().as_ref(),
                "-frames:v",
                "1",
                "-q:v",
                "5", // Lower quality (sufficient for semantic encoding, faster)
                output_path.to_string_lossy().as_ref(),
            ])
            .output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| {
        CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e))
    })?;

    if !output.status.success() {
        return Err(CacheError::VideoProcessingError(format!(
            "ffmpeg exited with status: {}",
            output.status
        )));
    }

    Ok(())
}

/// Extract multiple frames from a video at specified times in a single ffmpeg call
/// This is significantly faster than calling extract_frame_at_time multiple times
pub async fn extract_frames_batch(
    video_path: &Path,
    frame_times: &[f64],
    output_dir: &Path,
) -> CacheResult<Vec<PathBuf>> {
    if frame_times.is_empty() {
        return Ok(Vec::new());
    }

    std::fs::create_dir_all(output_dir)?;

    let video_path = video_path.to_path_buf();
    let output_dir_path = output_dir.to_path_buf();
    let output_dir_clone = output_dir_path.clone();
    let ffmpeg_path = get_ffmpeg_path();
    let ffmpeg_path_for_err = ffmpeg_path.clone();
    let frame_times = frame_times.to_vec();
    let frame_count = frame_times.len();

    let output = tokio::task::spawn_blocking(move || {
        let mut args = vec!["-y".to_string()];

        // Add inputs with seeking
        for t in &frame_times {
            args.push("-ss".to_string());
            args.push(t.to_string());
            args.push("-i".to_string());
            args.push(video_path.to_string_lossy().into_owned());
        }

        // Map inputs to outputs
        for i in 0..frame_count {
            args.push("-map".to_string());
            args.push(format!("{}:v", i));
            args.push("-frames:v".to_string());
            args.push("1".to_string());
            args.push("-q:v".to_string());
            args.push("5".to_string());
            args.push("-strict".to_string());
            args.push("-1".to_string());
            args.push("-update".to_string());
            args.push("1".to_string());
            args.push("-vf".to_string());
            args.push("scale=224:224".to_string());
            args.push(
                output_dir_path
                    .join(format!("frame_{}.jpg", i))
                    .to_string_lossy()
                    .into_owned(),
            );
        }

        Command::new(ffmpeg_path).args(&args).output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| {
        CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CacheError::VideoProcessingError(format!(
            "ffmpeg batch extraction failed: {}. stderr: {}",
            output.status, stderr
        )));
    }

    // Return paths to extracted frames (only those that were successfully created)
    Ok((0..frame_count)
        .map(|i| output_dir_clone.join(format!("frame_{}.jpg", i)))
        .filter(|p| p.exists())
        .collect())
}

/// Check if a video uses HEVC codec
pub async fn is_hevc_video(video_path: &Path) -> CacheResult<bool> {
    let video_path = video_path.to_path_buf();
    let ffprobe_path = get_ffprobe_path();
    let ffprobe_path_for_err = ffprobe_path.clone();

    let output = tokio::task::spawn_blocking(move || {
        Command::new(ffprobe_path)
            .args([
                "-v",
                "quiet",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                video_path.to_string_lossy().as_ref(),
            ])
            .output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| {
        CacheError::VideoProcessingError(format_binary_error("ffprobe", &ffprobe_path_for_err, &e))
    })?;

    if !output.status.success() {
        return Err(CacheError::VideoProcessingError(format!(
            "ffprobe exited with status: {}",
            output.status
        )));
    }

    let codec = String::from_utf8(output.stdout)
        .map_err(|e| CacheError::VideoProcessingError(format!("Invalid UTF-8 output: {}", e)))?
        .trim()
        .to_lowercase();

    Ok(codec == "hevc" || codec == "h265")
}

fn parse_root_atom_offset(trace: &str, atom: &str) -> Option<u64> {
    let marker = format!("type:'{}' parent:'root'", atom);

    trace.lines().find_map(|line| {
        if !line.contains(&marker) {
            return None;
        }

        let (_, size_part) = line.split_once("sz:")?;
        size_part.split_whitespace().nth(1)?.parse::<u64>().ok()
    })
}

pub fn has_moov_at_start(path: &Path) -> CacheResult<bool> {
    let ffprobe_path = get_ffprobe_path();
    let output = Command::new(&ffprobe_path)
        .args(["-v", "trace", path.to_string_lossy().as_ref()])
        .output()
        .map_err(|e| {
            CacheError::VideoProcessingError(format_binary_error("ffprobe", &ffprobe_path, &e))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CacheError::VideoProcessingError(format!(
            "ffprobe exited with status {}. stderr: {}",
            output.status, stderr
        )));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let moov_offset = parse_root_atom_offset(&stderr, "moov");
    let mdat_offset = parse_root_atom_offset(&stderr, "mdat");

    let is_at_start = match (moov_offset, mdat_offset) {
        (Some(moov), Some(mdat)) => moov < mdat || moov < 1000,
        (Some(moov), None) => moov < 1000,
        (None, _) => true,
    };

    Ok(is_at_start)
}

pub fn fix_moov_atom(path: &Path) -> CacheResult<()> {
    if has_moov_at_start(path)? {
        return Ok(());
    }

    let ffmpeg_path = get_ffmpeg_path();
    let parent = path.parent().ok_or_else(|| {
        CacheError::VideoProcessingError(format!("Path has no parent: {}", path.display()))
    })?;
    let file_stem = path.file_stem().and_then(|n| n.to_str()).ok_or_else(|| {
        CacheError::VideoProcessingError(format!("Invalid file name: {}", path.display()))
    })?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let temp_path = parent.join(format!(
        "{}.moovfix.{}.{}",
        file_stem,
        std::process::id(),
        extension
    ));

    let output = Command::new(&ffmpeg_path)
        .args([
            "-y",
            "-i",
            path.to_string_lossy().as_ref(),
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            temp_path.to_string_lossy().as_ref(),
        ])
        .output()
        .map_err(|e| {
            CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path, &e))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_file(&temp_path);
        return Err(CacheError::VideoProcessingError(format!(
            "ffmpeg faststart remux exited with status {}. stderr: {}",
            output.status, stderr
        )));
    }

    std::fs::rename(&temp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        CacheError::VideoProcessingError(format!(
            "Failed to atomically replace video {}: {}",
            path.display(),
            e
        ))
    })?;

    Ok(())
}

/// Serve-time streamability fix: remux an MP4 with the moov atom moved to the
/// front via a fast `-c copy -movflags +faststart` pass, so browsers can start
/// progressive playback immediately. This is a cheap stream copy (no re-encode,
/// no decoder), separate from the full HEVC transcode path. No-op (returns
/// `Ok(())`) when the input already has moov at the start or the sidecar
/// already exists. Writes to a unique temp file then atomically renames into
/// place, so concurrent requests for the same sidecar cannot interleave into
/// a corrupt output.
pub async fn ensure_progressive_mp4(input_path: &Path, output_path: &Path) -> CacheResult<()> {
    // Bound the whole remux (moov probe + ffmpeg copy) by the remux semaphore
    // so a burst of NeedsRemux requests cannot spawn unbounded blocking
    // ffprobe/ffmpeg processes on the async runtime.
    let _permit = get_remox_semaphore().acquire().await.map_err(|e| {
        CacheError::VideoProcessingError(format!(
            "Failed to acquire remux semaphore for {}: {}",
            output_path.display(),
            e
        ))
    })?;

    // Re-check under the permit: another request may have completed the remux
    // while this one queued on the semaphore.
    if output_path.exists() {
        return Ok(());
    }

    // `has_moov_at_start` runs a blocking ffprobe; offload it so it cannot
    // stall a tokio worker thread.
    let probe_input = input_path.to_path_buf();
    let moov_at_start = tokio::task::spawn_blocking(move || has_moov_at_start(&probe_input))
        .await
        .map_err(|e| CacheError::VideoProcessingError(format!("ffprobe task panicked: {e}")))??;
    if moov_at_start {
        return Ok(());
    }

    // Unique temp path: the remux path has no per-hash claim (unlike the
    // transcode path), so a fixed name would let two concurrent remuxes of the
    // same sidecar write the same file. Named in the SAME directory as the
    // final path so the completed file can be atomically renamed in.
    let temp_output_path = remux_temp_path(output_path);
    let output_path_owned = output_path.to_path_buf();

    // Create output directory if it doesn't exist.
    if let Some(parent) = temp_output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CacheError::VideoProcessingError(format!("Failed to create output directory: {}", e))
        })?;
    }

    let ffmpeg_path = get_ffmpeg_path();
    let mut command = TokioCommand::new(&ffmpeg_path);
    command.kill_on_drop(true).args([
        "-y",
        "-i",
        input_path.to_string_lossy().as_ref(),
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        // Force the muxer explicitly: the temp path ends in `.tmp`, so ffmpeg
        // cannot infer the format from the extension.
        "-f",
        "mp4",
        temp_output_path.to_string_lossy().as_ref(),
    ]);

    let output = command.output().await.map_err(|e| {
        CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path, &e))
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_file(&temp_output_path);
        return Err(CacheError::VideoProcessingError(format!(
            "ffmpeg faststart remux exited with status {}. stderr: {}",
            output.status, stderr
        )));
    }

    // Move the completed temp file into place (atomic on the same filesystem).
    std::fs::rename(&temp_output_path, &output_path_owned).map_err(|e| {
        let _ = std::fs::remove_file(&temp_output_path);
        CacheError::VideoProcessingError(format!("Failed to move remuxed video into place: {}", e))
    })?;

    Ok(())
}

/// A per-call unique temp path in the same directory as `output_path`, so two
/// concurrent remuxes of the same sidecar never write the same file.
fn remux_temp_path(output_path: &Path) -> PathBuf {
    let seq = REMUX_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("remux");
    parent.join(format!("{stem}.{}.{}.tmp", std::process::id(), seq))
}
/// Transcode any video codec to H.264 for browser compatibility.
pub async fn transcode_codec_to_h264(input_path: &Path, output_path: &Path) -> CacheResult<()> {
    transcode_codec_to_h264_with_timeout(
        input_path,
        output_path,
        Duration::from_secs(transcode_timeout_secs()),
        None,
    )
    .await
}

/// Transcode any video codec to H.264, reporting progress percentage to
/// `on_progress` as ffmpeg emits `-progress pipe:1` lines. `on_progress` is
/// called with `Some(percent)` (0..=100) whenever a progress line arrives and
/// the input duration is known, and never with a decreasing value. When the
/// duration is unknown, `on_progress(None)` signals "working, no percent".
pub async fn transcode_codec_to_h264_with_progress(
    input_path: &Path,
    output_path: &Path,
    on_progress: Arc<dyn Fn(Option<u8>) + Send + Sync>,
) -> CacheResult<()> {
    transcode_codec_to_h264_with_timeout(
        input_path,
        output_path,
        Duration::from_secs(transcode_timeout_secs()),
        Some(on_progress),
    )
    .await
}

async fn transcode_codec_to_h264_with_timeout(
    input_path: &Path,
    output_path: &Path,
    timeout_duration: Duration,
    on_progress: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
) -> CacheResult<()> {
    let ffmpeg_path = get_ffmpeg_path();
    transcode_codec_to_h264_with_timeout_and_path(
        input_path,
        output_path,
        timeout_duration,
        ffmpeg_path,
        on_progress,
    )
    .await
}

/// Progress state shared between the stdout-reading task and the error path.
struct ProgressParser {
    /// Input duration in seconds when known (drives percent computation).
    duration: Option<f64>,
    /// Most recent percent already reported, so values never regress.
    last_percent: u8,
    /// Callback (None = no progress reporting requested).
    on_progress: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
}

impl ProgressParser {
    /// Handle one `-progress pipe:1` `key=value` line, reporting a percent.
    /// ffmpeg progress emits `out_time_us` (microseconds) and `out_time_ms`
    /// (milliseconds); either is accepted via the task's preferred unit.
    fn handle_line(&mut self, line: &str) {
        let Some((key, value)) = line.trim().split_once('=') else {
            return;
        };
        let seconds = match key {
            "out_time_us" => value.parse::<f64>().ok().map(|us| us / 1_000_000.0),
            "out_time_ms" => value.parse::<f64>().ok().map(|ms| ms / 1_000.0),
            _ => return,
        };
        let (Some(seconds), Some(duration)) = (seconds, self.duration) else {
            return;
        };
        if duration <= 0.0 || !duration.is_finite() {
            return;
        }
        let percent = ((seconds / duration * 100.0).round() as u8)
            .clamp(0, 100)
            .min(100);
        if percent > self.last_percent {
            self.last_percent = percent;
            if let Some(cb) = &self.on_progress {
                cb(Some(percent));
            }
        }
    }

    fn signal_unknown(&self) {
        if let Some(cb) = &self.on_progress {
            cb(None);
        }
    }
}

async fn transcode_codec_to_h264_with_timeout_and_path(
    input_path: &Path,
    output_path: &Path,
    timeout_duration: Duration,
    ffmpeg_path: String,
    on_progress: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
) -> CacheResult<()> {
    // Write to a temp file in the SAME directory as the final path so the
    // completed file can be atomically renamed into place. A failed or
    // timed-out transcode must never leave a partial file at `output_path`,
    // which callers would otherwise treat as valid video.
    let temp_output_path = output_path.with_extension("mp4.tmp");
    let output_path_owned = output_path.to_path_buf();

    let inner = async {
        let _permit = acquire_transcode_permit().await?;

        // Create output directory if it doesn't exist
        if let Some(parent) = temp_output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CacheError::VideoProcessingError(format!(
                    "Failed to create output directory: {}",
                    e
                ))
            })?;
        }

        // Probe the input duration once so progress can be expressed as a
        // percentage. A best-effort probe: if it fails, percent is unknown and
        // the client is told "working" with no number.
        let duration = if on_progress.is_some() {
            extract_video_metadata(input_path)
                .await
                .ok()
                .map(|m| m.duration)
        } else {
            None
        };
        let mut progress = ProgressParser {
            duration,
            last_percent: 0,
            on_progress,
        };
        if duration.is_none() {
            progress.signal_unknown();
        }

        // Try hardware-accelerated decoding first, fall back to software if
        // unavailable. `-progress pipe:1` streams key=value progress lines to
        // stdout, which we read incrementally to report percent.
        let ffmpeg_path_for_err = ffmpeg_path.clone();
        let mut command = TokioCommand::new(ffmpeg_path);
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args([
                "-hwaccel",
                "auto", // Auto-detect hardware acceleration (VAAPI, NVDEC, etc.)
                "-i",
                input_path.to_string_lossy().as_ref(),
                "-c:v",
                "libx264", // Use H.264 encoder (more widely available than libopenh264)
                "-preset",
                "fast", // Encoding speed preset (fast is good for real-time transcoding)
                "-crf",
                "23", // Constant Rate Factor (18-28, lower = better quality)
                "-c:a",
                "copy", // Copy audio stream without re-encoding (faster)
                "-movflags",
                "+faststart", // Enable streaming-friendly format
                "-y",         // Overwrite output file
                "-progress",
                "pipe:1", // Stream progress to stdout for percent reporting
                // Force the muxer explicitly: the temp output path ends in
                // `.mp4.tmp`, so ffmpeg cannot infer the format from the
                // extension and otherwise fails with "Error initializing the
                // muxer: Invalid argument".
                "-f",
                "mp4",
                temp_output_path.to_string_lossy().as_ref(),
            ]);

        let mut child = command.spawn().map_err(|e| {
            CacheError::VideoProcessingError(format_binary_error(
                "ffmpeg",
                &ffmpeg_path_for_err,
                &e,
            ))
        })?;

        // Drain stdout (progress) incrementally; collect stderr for the error
        // message so a long ffmpeg run cannot deadlock on a full pipe.
        let stdout = child.stdout.take().ok_or_else(|| {
            CacheError::VideoProcessingError("ffmpeg stdout pipe unavailable".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            CacheError::VideoProcessingError("ffmpeg stderr pipe unavailable".to_string())
        })?;

        let mut stderr_reader = BufReader::new(stderr);
        let stderr_handle = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = stderr_reader.read_to_string(&mut buf).await;
            buf
        });

        let mut stdout_reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match stdout_reader.read_line(&mut line).await {
                Ok(0) => break, // EOF: ffmpeg closed stdout
                Ok(_) => progress.handle_line(&line),
                Err(e) => {
                    // A read error on the progress pipe is non-fatal: the
                    // transcode's real outcome comes from the exit status.
                    log::debug!("ffmpeg progress pipe read error: {e}");
                    break;
                }
            }
        }

        // Wait for ffmpeg to finish, then join the stderr collector.
        let status = child.wait().await.map_err(|e| {
            CacheError::VideoProcessingError(format_binary_error(
                "ffmpeg",
                &ffmpeg_path_for_err,
                &e,
            ))
        })?;
        let stderr = stderr_handle.await.unwrap_or_default();

        if !status.success() {
            log::error!("FFmpeg transcoding failed!");
            log::error!("FFmpeg stderr: {}", stderr);
            let _ = std::fs::remove_file(&temp_output_path);
            return Err(CacheError::VideoProcessingError(format!(
                "ffmpeg transcode exited with status {}. stderr: {}",
                status, stderr
            )));
        }

        // Move the completed temp file into place (atomic on the same filesystem).
        std::fs::rename(&temp_output_path, &output_path_owned).map_err(|e| {
            let _ = std::fs::remove_file(&temp_output_path);
            CacheError::VideoProcessingError(format!(
                "Failed to move transcoded video into place: {}",
                e
            ))
        })?;

        Ok::<(), CacheError>(())
    };

    match timeout(timeout_duration, inner).await {
        Ok(result) => result,
        Err(_) => {
            // The inner future (and with it the ffmpeg child, via kill_on_drop)
            // has been dropped; remove whatever partial output it wrote.
            let _ = std::fs::remove_file(&temp_output_path);
            Err(CacheError::VideoProcessingError(format!(
                "Transcoding timed out after {}s",
                timeout_duration.as_secs()
            )))
        }
    }
}

/// Get the path for a transcoded video in the cache
pub fn get_transcoded_path(cache_dir: &Path, original_hash: &str) -> PathBuf {
    let base = if cache_dir.file_name().is_some_and(|n| n == "transcoded") {
        cache_dir.to_path_buf()
    } else {
        cache_dir.join("transcoded")
    };
    base.join(format!("{}.mp4", original_hash))
}

/// Transcode cache path versioned by the source's content fingerprint (file
/// size + mtime millis). The DB hash is derived from the file PATH, so an
/// in-place edit keeps the hash while the bytes change — the version makes
/// the cache miss after the rescan notices the edit instead of serving the
/// stale H.264 transcode forever. Only one version file is kept per hash:
/// the transcode task removes older `{hash}_*.mp4` siblings on success.
pub fn get_transcoded_path_versioned(
    cache_dir: &Path,
    original_hash: &str,
    file_size: i64,
    modified_millis: i64,
) -> PathBuf {
    let base = if cache_dir.file_name().is_some_and(|n| n == "transcoded") {
        cache_dir.to_path_buf()
    } else {
        cache_dir.join("transcoded")
    };
    base.join(format!(
        "{}_{}_{}.mp4",
        original_hash, file_size, modified_millis
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::{CacheConfig, Config};
    use crate::db::{create_in_memory_pool, Photo};
    use crate::thumbnail_generator::ThumbnailGenerator;
    use crate::thumbnail_types::{ThumbnailFormat, ThumbnailSize};
    use chrono::Utc;
    use std::cell::Cell;
    use std::io::{Error, ErrorKind};
    use std::process::Command;
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
    use std::time::Duration;
    use tempfile::TempDir;

    thread_local! {
        static TEST_ENV_LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
    }

    // Shared test env lock: serializes FFPROBE_PATH/FFMPEG_PATH mutation across
    // test modules (handlers_video, metadata_extractor) so ffprobe-dependent
    // tests never observe another test's fake binary path.
    pub(crate) fn test_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    // Guard returned by acquire_test_env_lock: holding it on the outermost call
    // owns the mutex, and its Drop decrements the nesting depth so later tests on
    // the same thread can acquire the lock again. Without the drop-decrement the
    // depth would leak and every subsequent acquire on that thread would silently
    // return without locking.
    pub(crate) struct TestEnvGuard {
        _mutex: Option<MutexGuard<'static, ()>>,
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            TEST_ENV_LOCK_DEPTH.with(|depth| {
                depth.set(depth.get().saturating_sub(1));
            });
        }
    }

    pub(crate) fn acquire_test_env_lock() -> TestEnvGuard {
        TEST_ENV_LOCK_DEPTH.with(|depth| {
            let current = depth.get();
            depth.set(current + 1);

            TestEnvGuard {
                // Recover from a poisoned mutex: one panicking env-dependent
                // test must not cascade failures across every other test that
                // shells out to ffprobe/ffmpeg.
                _mutex: (current == 0).then(|| {
                    test_env_lock()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                }),
            }
        })
    }

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

    #[test]
    fn test_env_lock_guard_drop_resets_nesting_depth() {
        // GIVEN a guard acquired and released on this thread (incl. a nested acquire)
        {
            let _outer = acquire_test_env_lock();
            let _nested = acquire_test_env_lock();
        }
        // WHEN a fresh guard is acquired after the previous ones dropped
        let fresh = acquire_test_env_lock();
        // THEN the nesting depth was reset and the fresh guard actually owns the mutex
        // (a leaked depth would return a guard that holds nothing -> try_lock succeeds)
        assert!(
            test_env_lock().try_lock().is_err(),
            "fresh guard must hold the mutex after previous guards dropped"
        );
        drop(fresh);
    }

    fn project_photo_path(filename: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-data")
            .join(filename)
    }

    fn has_command(cmd: &str) -> bool {
        std::process::Command::new(cmd)
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn should_run_video_tests(filename: &str) -> bool {
        let run_var = std::env::var("RUN_VIDEO_TESTS").unwrap_or_default();
        if !(run_var == "1" || run_var.eq_ignore_ascii_case("true")) {
            eprintln!("RUN_VIDEO_TESTS not set to '1' or 'true'; skipping video tests");
            return false;
        }

        let path = project_photo_path(filename);
        if !path.exists() {
            eprintln!(
                "Required test video not found at {}; skipping video tests",
                path.display()
            );
            return false;
        }

        if !has_command("ffprobe") {
            eprintln!("ffprobe not found in PATH; skipping video tests");
            return false;
        }

        if !has_command("ffmpeg") {
            eprintln!("ffmpeg not found in PATH; skipping video tests");
            return false;
        }

        true
    }

    const TEST_PORT: u16 = 18473;

    #[test]
    fn test_verify_ffmpeg_available_fails_not_found() {
        // GIVEN missing ffmpeg and ffprobe paths
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", "/nonexistent/ffmpeg");
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

        // WHEN ffmpeg availability is verified
        let result = verify_ffmpeg_available();

        // THEN the error reports the missing ffmpeg binary path
        let error = result.expect_err("expected ffmpeg verification to fail");
        assert!(error.contains("not found at"), "unexpected error: {error}");
        assert!(
            error.contains("/nonexistent/ffmpeg"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_verify_ffmpeg_available_fails_bad_ffprobe() {
        // GIVEN a valid ffmpeg binary and a missing ffprobe path
        let temp_dir = TempDir::new().unwrap();
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg_ok.sh");
        std::fs::write(&ffmpeg_script, "#!/usr/bin/env sh\nexit 0\n").unwrap();
        make_executable(&ffmpeg_script);

        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

        // WHEN ffmpeg availability is verified
        let result = verify_ffmpeg_available();

        // THEN the error reports the missing ffprobe binary path
        let error = result.expect_err("expected ffprobe verification to fail");
        assert!(error.contains("ffprobe"), "unexpected error: {error}");
        assert!(error.contains("not found"), "unexpected error: {error}");
    }

    #[test]
    fn test_format_binary_error_not_found() {
        // GIVEN a not found IO error
        let error = Error::new(ErrorKind::NotFound, "No such file or directory");

        // WHEN the binary error is formatted
        let message = format_binary_error("ffprobe", "/bad/path", &error);

        // THEN the message reports the missing binary path
        assert!(
            message.contains("not found at"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/bad/path"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn test_format_binary_error_other_error() {
        // GIVEN a non-not-found IO error
        let error = Error::new(ErrorKind::PermissionDenied, "Permission denied");

        // WHEN the binary error is formatted
        let message = format_binary_error("ffmpeg", "/bad/path", &error);

        // THEN the message reports execution failure details
        assert!(
            message.contains("failed to execute"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/bad/path"),
            "unexpected message: {message}"
        );
    }

    fn create_test_video_with_movflags(source: &Path, destination: &Path, movflags: &str) {
        let output = Command::new("ffmpeg")
            .args([
                "-y",
                "-i",
                source.to_str().unwrap(),
                "-c",
                "copy",
                "-movflags",
                movflags,
                destination.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!(
                "Failed to create test video with movflags {}: {}",
                movflags, stderr
            );
        }
    }
    // Force a moov-at-end copy of a valid mp4: `-movflags -faststart` DISABLES
    // faststart, leaving the moov atom at the end of the file, which is the
    // intended "broken" input for the progressive-remux test.
    fn ffmpeg_copy_moov_end(src: &Path, dst: &Path) {
        let output = Command::new(get_ffmpeg_path())
            .args([
                "-y",
                "-i",
                src.to_string_lossy().as_ref(),
                "-c",
                "copy",
                "-movflags",
                "-faststart",
                "-f",
                "mp4",
                dst.to_string_lossy().as_ref(),
            ])
            .output()
            .unwrap();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("Failed to create moov-at-end test video: {}", stderr);
        }
    }

    #[test]
    fn test_moov_detection() {
        let video_filename = "test_video.mp4";
        if !should_run_video_tests(video_filename) {
            eprintln!("Skipping MOOV detection test (prereqs missing or RUN_VIDEO_TESTS not set)");
            return;
        }

        let _env_lock = acquire_test_env_lock();

        let temp_dir = TempDir::new().unwrap();
        let source = project_photo_path(video_filename);
        let moov_start = temp_dir.path().join("moov_start.mp4");
        let moov_end = temp_dir.path().join("moov_end.mp4");

        create_test_video_with_movflags(&source, &moov_start, "+faststart");
        create_test_video_with_movflags(&source, &moov_end, "-faststart");

        assert!(has_moov_at_start(&moov_start).unwrap());
        assert!(!has_moov_at_start(&moov_end).unwrap());
    }

    #[test]
    fn test_moov_fix() {
        let video_filename = "test_video.mp4";
        if !should_run_video_tests(video_filename) {
            eprintln!("Skipping MOOV fix test (prereqs missing or RUN_VIDEO_TESTS not set)");
            return;
        }

        let _env_lock = acquire_test_env_lock();

        let temp_dir = TempDir::new().unwrap();
        let source = project_photo_path(video_filename);
        let moov_end = temp_dir.path().join("moov_end.mp4");

        create_test_video_with_movflags(&source, &moov_end, "-faststart");

        assert!(!has_moov_at_start(&moov_end).unwrap());
        fix_moov_atom(&moov_end).unwrap();
        assert!(has_moov_at_start(&moov_end).unwrap());
    }

    #[test]
    fn test_moov_skip_if_ok() {
        let video_filename = "test_video.mp4";
        if !should_run_video_tests(video_filename) {
            eprintln!("Skipping MOOV skip test (prereqs missing or RUN_VIDEO_TESTS not set)");
            return;
        }

        let _env_lock = acquire_test_env_lock();

        let temp_dir = TempDir::new().unwrap();
        let source = project_photo_path(video_filename);
        let moov_start = temp_dir.path().join("moov_start.mp4");

        create_test_video_with_movflags(&source, &moov_start, "+faststart");

        let before = std::fs::metadata(&moov_start).unwrap().modified().unwrap();
        fix_moov_atom(&moov_start).unwrap();
        let after = std::fs::metadata(&moov_start).unwrap().modified().unwrap();

        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn ensure_progressive_mp4_remuxes_moov_to_front() {
        let _lock = acquire_test_env_lock();
        let temp = TempDir::new().unwrap();
        // Real fixture has moov at start already; force a moov-at-end copy.
        let src = Path::new("test-data/test_video.mp4");
        if !src.exists() {
            return;
        }
        let moov_end = temp.path().join("in.mp4");
        ffmpeg_copy_moov_end(src, &moov_end); // local test helper defined below
        assert!(!has_moov_at_start(&moov_end).unwrap());
        let out = temp.path().join("out.mp4");
        ensure_progressive_mp4(&moov_end, &out).await.unwrap();
        assert!(out.exists());
        assert!(
            has_moov_at_start(&out).unwrap(),
            "remux must move moov forward"
        );
        // second call is idempotent on a start-front file
        ensure_progressive_mp4(&out, &out).await.unwrap();
    }

    fn create_test_config() -> (Config, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("cache");

        let data_path = temp_dir.path().to_string_lossy().to_string();
        let db_path = temp_dir
            .path()
            .join("database/turbo-pix.db")
            .to_string_lossy()
            .to_string();

        let config = Config {
            host: "127.0.0.1".to_string(),
            allowed_hosts: vec![],
            port: TEST_PORT,
            photo_paths: vec![],
            data_path,
            db_path,
            cache: CacheConfig {
                thumbnail_cache_path: cache_path.join("thumbnails").to_string_lossy().to_string(),
                max_cache_size_mb: 1024,
            },
            transcode_timeout_secs: 300,
            locale: "en".to_string(),
            nominatim_url: "https://nominatim.openstreetmap.org".to_string(),
        };

        (config, temp_dir)
    }

    fn create_test_video_photo(path: &str) -> Photo {
        let now = Utc::now();
        Photo {
            hash_sha256: "b".repeat(64),
            file_path: path.to_string(),
            filename: "test_video.mp4".to_string(),
            file_size: 11156,
            mime_type: Some("video/mp4".to_string()),
            taken_at: Some(now),
            width: Some(1920),
            height: Some(1080),
            orientation: Some(1),
            duration: Some(0.3),
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: serde_json::json!({
                "settings": {
                    "flash_used": false
                },
                "video": {
                    "codec": "h264",
                    "audio_codec": "aac",
                    "bitrate": 1000,
                    "frame_rate": 30.0
                }
            }),
            date_modified: now,
            date_indexed: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn test_video_thumbnail_generation() {
        let (config, _temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().await.unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        let video_filename = "test_video.mp4";
        let video_path = project_photo_path(video_filename);
        if !should_run_video_tests(video_filename) {
            eprintln!("Skipping video thumbnail generation test (prereqs missing or RUN_VIDEO_TESTS not set)");
            return;
        }
        let _env_lock = acquire_test_env_lock();
        let video_path_str = video_path.to_string_lossy().into_owned();
        let photo = create_test_video_photo(&video_path_str);

        let result = generator
            .get_or_generate(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg)
            .await;

        assert!(result.is_ok(), "Video thumbnail generation should succeed");

        let thumbnail_data = result.unwrap();
        assert!(
            !thumbnail_data.is_empty(),
            "Thumbnail data should not be empty"
        );
        assert!(
            thumbnail_data.len() > 1000,
            "Thumbnail should be a reasonable size (>1KB)"
        );

        let cache_key = crate::thumbnail_types::CacheKey::from_photo(
            &photo,
            ThumbnailSize::Medium,
            ThumbnailFormat::Jpeg,
        )
        .unwrap();
        let cache_path = generator.get_cache_path(&cache_key);
        assert!(cache_path.exists(), "Thumbnail should be cached on disk");
    }

    #[tokio::test]
    async fn test_video_metadata_extraction() {
        let video_filename = "test_video.mp4";
        let video_path = project_photo_path(video_filename);
        if !should_run_video_tests(video_filename) {
            eprintln!("Skipping video metadata extraction test (prereqs missing or RUN_VIDEO_TESTS not set)");
            return;
        }
        let _env_lock = acquire_test_env_lock();
        let metadata = extract_video_metadata(&video_path).await;

        assert!(
            metadata.is_ok(),
            "Should extract video metadata successfully"
        );
        let metadata = metadata.unwrap();

        assert!(metadata.duration > 0.0, "Duration should be positive");
        assert_eq!(metadata.width, 1920, "Width should match expected");
        assert_eq!(metadata.height, 1080, "Height should match expected");
    }

    #[tokio::test]
    async fn test_video_frame_timing_calculation() {
        let short_video = VideoMetadata {
            duration: 2.0,
            width: 320,
            height: 240,
        };
        let medium_video = VideoMetadata {
            duration: 30.0,
            width: 320,
            height: 240,
        };
        let long_video = VideoMetadata {
            duration: 3600.0,
            width: 320,
            height: 240,
        };

        let short_time = calculate_optimal_frame_time(&short_video);
        let medium_time = calculate_optimal_frame_time(&medium_video);
        let long_time = calculate_optimal_frame_time(&long_video);

        assert!(short_time >= 0.5, "Should not extract before 0.5 seconds");
        assert!(short_time <= 2.0, "Should not exceed video duration");

        assert!(medium_time >= 0.5, "Should not extract before 0.5 seconds");
        assert!(medium_time <= 30.0, "Should not exceed video duration");

        assert!(long_time >= 0.5, "Should not extract before 0.5 seconds");
        assert!(
            long_time <= 30.0,
            "Should cap at 30 seconds for long videos"
        );
    }

    #[tokio::test]
    async fn test_video_thumbnail_different_sizes() {
        let (config, _temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().await.unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        let video_filename = "test_video.mp4";
        let video_path = project_photo_path(video_filename);
        if !should_run_video_tests(video_filename) {
            eprintln!("Skipping video thumbnail different sizes test (prereqs missing or RUN_VIDEO_TESTS not set)");
            return;
        }
        let _env_lock = acquire_test_env_lock();
        let video_path_str = video_path.to_string_lossy().into_owned();
        let photo = create_test_video_photo(&video_path_str);

        let small = generator
            .get_or_generate(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg)
            .await
            .unwrap();
        let medium = generator
            .get_or_generate(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg)
            .await
            .unwrap();
        let large = generator
            .get_or_generate(&photo, ThumbnailSize::Large, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        assert!(!small.is_empty());
        assert!(!medium.is_empty());
        assert!(!large.is_empty());

        assert!(medium.len() >= small.len(), "Medium should be >= small");
        assert!(large.len() >= medium.len(), "Large should be >= medium");
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[tokio::test]
    async fn test_transcode_disabled_rejects_permit() {
        // TURBO_PIX_MAX_TRANSCODES=0 disables transcoding. acquire checks
        // transcode_max_pool() == 0 BEFORE acquiring the (possibly already
        // initialized) semaphore, so this is deterministic.
        let _env = EnvVarGuard::set("TURBO_PIX_MAX_TRANSCODES", "0");
        let err = acquire_transcode_permit().await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("disabled"),
            "expected disabled error, got: {}",
            msg
        );
    }

    #[test]
    fn test_transcode_max_pool_parses_env() {
        let _env = EnvVarGuard::set("TURBO_PIX_MAX_TRANSCODES", "4");
        assert_eq!(transcode_max_pool(), 4);
        let _env2 = EnvVarGuard::set("TURBO_PIX_MAX_TRANSCODES", "0");
        assert_eq!(transcode_max_pool(), 0, "0 must mean disabled");
    }

    #[tokio::test]
    async fn test_transcode_reports_percent_from_progress_lines() {
        let _lock = acquire_test_env_lock();
        let temp_dir = TempDir::new().unwrap();

        // Fake ffprobe reports a 10s duration (needed to turn out_time into %).
        let ffprobe_script = temp_dir.path().join("fake_ffprobe_duration.sh");
        std::fs::write(
            &ffprobe_script,
            "#!/usr/bin/env sh\nprintf '%s\\n' '{\"format\":{\"duration\":\"10.0\"},\"streams\":[{\"codec_type\":\"video\",\"codec_name\":\"h264\",\"width\":320,\"height\":240}]}'",
        )
        .unwrap();
        make_executable(&ffprobe_script);

        // Fake ffmpeg writes progress lines (emulating ~30% then ~70% complete),
        // touches the output file, and exits 0.
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg_progress.sh");
        std::fs::write(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\nprintf '%s\\n' 'out_time_us=3000000' 'progress=continue' 'out_time_us=7000000' 'progress=continue' > /dev/stdout\ntouch \"$last\"\nexit 0\n",
        )
        .unwrap();
        make_executable(&ffmpeg_script);

        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());

        let input = temp_dir.path().join("input.mp4");
        let output = temp_dir.path().join("output.mp4");
        std::fs::write(&input, b"not-a-real-video").unwrap();

        let reported: Arc<Mutex<Vec<Option<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let cb = reported.clone();
        let on_progress = Arc::new(move |p: Option<u8>| {
            cb.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(p);
        });

        transcode_codec_to_h264_with_progress(&input, &output, on_progress)
            .await
            .expect("transcode should succeed");

        let reported = reported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 3s / 10s = 30%, then 7s / 10s = 70%.
        assert!(
            reported.contains(&Some(30)),
            "expected 30% progress callback, got: {:?}",
            *reported
        );
        assert!(
            reported.contains(&Some(70)),
            "expected 70% progress callback, got: {:?}",
            *reported
        );
    }

    #[tokio::test]
    async fn test_transcode_timeout() {
        let temp_dir = TempDir::new().unwrap();
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg_timeout.sh");
        // Write a partial output file, then sleep past the timeout so the
        // transcode is killed mid-write.
        std::fs::write(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\necho partial > \"$last\"\nsleep 2\nexit 0\n",
        )
        .unwrap();
        make_executable(&ffmpeg_script);

        let input = temp_dir.path().join("input.mp4");
        let output = temp_dir.path().join("output.mp4");
        std::fs::write(&input, b"not-a-real-video").unwrap();

        let result = transcode_codec_to_h264_with_timeout_and_path(
            &input,
            &output,
            Duration::from_secs(1),
            ffmpeg_script.to_str().unwrap().to_string(),
            None,
        )
        .await;

        assert!(result.is_err(), "Expected timeout error");
        let error = format!("{}", result.unwrap_err());
        assert!(
            error.contains("timed out"),
            "Error should mention timeout, got: {}",
            error
        );
        // The partial file written before the timeout must not survive, neither
        // at the final path nor at the temp path.
        assert!(
            !output.exists(),
            "partial output must be cleaned up on timeout"
        );
        assert!(
            !output.with_extension("mp4.tmp").exists(),
            "temp file must be cleaned up on timeout"
        );
    }

    #[tokio::test]
    async fn test_transcode_failure_cleans_partial_file() {
        let temp_dir = TempDir::new().unwrap();
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg_fail.sh");
        // Simulate ffmpeg failing mid-write: create a partial output file and
        // exit non-zero.
        std::fs::write(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\necho partial > \"$last\"\nexit 1\n",
        )
        .unwrap();
        make_executable(&ffmpeg_script);

        let input = temp_dir.path().join("input.mp4");
        let output = temp_dir.path().join("output.mp4");
        std::fs::write(&input, b"not-a-real-video").unwrap();

        let result = transcode_codec_to_h264_with_timeout_and_path(
            &input,
            &output,
            Duration::from_secs(5),
            ffmpeg_script.to_str().unwrap().to_string(),
            None,
        )
        .await;

        assert!(result.is_err(), "Expected transcode failure");
        assert!(
            !output.exists(),
            "partial output must not be left at the final path on failure"
        );
        assert!(
            !output.with_extension("mp4.tmp").exists(),
            "temp file must be cleaned up on failure"
        );
    }

    #[tokio::test]
    async fn test_transcode_happy_path() {
        let temp_dir = TempDir::new().unwrap();
        let args_file = temp_dir.path().join("args.txt");
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg_ok.sh");
        std::fs::write(
            &ffmpeg_script,
            format!(
                "#!/usr/bin/env sh\nfor last; do :; done\nprintf '%s\\n' \"$@\" > '{}'\ntouch \"$last\"\nexit 0\n",
                args_file.display()
            ),
        )
        .unwrap();
        make_executable(&ffmpeg_script);

        let input = temp_dir.path().join("input.mp4");
        let output = temp_dir.path().join("nested/output.mp4");
        std::fs::write(&input, b"not-a-real-video").unwrap();

        let result = transcode_codec_to_h264_with_timeout_and_path(
            &input,
            &output,
            Duration::from_secs(5),
            ffmpeg_script.to_str().unwrap().to_string(),
            None,
        )
        .await;

        assert!(
            result.is_ok(),
            "Expected transcode to succeed: {:?}",
            result
        );
        assert!(output.exists(), "Expected output file to be created");
        assert!(
            !output.with_extension("mp4.tmp").exists(),
            "temp file must be renamed away on success"
        );

        // Regression: the temp output path ends in `.mp4.tmp`, so ffmpeg
        // cannot infer the format from the extension — the command must pass
        // `-f mp4` explicitly or the real ffmpeg fails with "Error
        // initializing the muxer: Invalid argument".
        let recorded_args = std::fs::read_to_string(&args_file).unwrap();
        let args: Vec<&str> = recorded_args.lines().collect();
        let f_index = args
            .iter()
            .position(|a| *a == "-f")
            .unwrap_or_else(|| panic!("ffmpeg args missing `-f` flag: {:?}", args));
        assert_eq!(
            args.get(f_index + 1).copied(),
            Some("mp4"),
            "ffmpeg must be told the mp4 muxer explicitly"
        );
    }

    #[test]
    fn test_transcode_status_json() {
        let status = TranscodeStatus {
            state: TranscodeState::InProgress,
            hash: "abc".to_string(),
            started_at: None,
            error: None,
            percent: None,
        };

        let json = serde_json::to_string(&status).expect("JSON serialization failed");
        assert!(
            json.contains("\"state\":\"InProgress\""),
            "JSON should contain InProgress state, got: {}",
            json
        );
        assert!(
            json.contains("\"hash\":\"abc\""),
            "JSON should contain hash abc, got: {}",
            json
        );
    }

    #[tokio::test]
    async fn test_error_message_not_found_ffprobe_extract_metadata() {
        // GIVEN a nonexistent ffprobe path
        let _guard = EnvVarGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

        // WHEN extract_video_metadata is called
        let result = extract_video_metadata(Path::new("/any/path")).await;

        // THEN the error message reports "not found at" with the path
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("not found at"),
            "expected 'not found at' in: {err_str}"
        );
        assert!(
            err_str.contains("/nonexistent/ffprobe"),
            "expected path in: {err_str}"
        );
    }

    #[tokio::test]
    async fn test_error_message_not_found_ffmpeg_extract_frame() {
        // GIVEN a nonexistent ffmpeg path
        let _guard = EnvVarGuard::set("FFMPEG_PATH", "/nonexistent/ffmpeg");

        // WHEN extract_frame_at_time is called
        let result =
            extract_frame_at_time(Path::new("/any/video"), 1.0, Path::new("/any/out")).await;

        // THEN the error message reports "not found at" with the path
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("not found at"),
            "expected 'not found at' in: {err_str}"
        );
        assert!(
            err_str.contains("/nonexistent/ffmpeg"),
            "expected path in: {err_str}"
        );
    }

    #[tokio::test]
    async fn test_error_message_not_found_ffprobe_is_hevc() {
        // GIVEN a nonexistent ffprobe path
        let _guard = EnvVarGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

        // WHEN is_hevc_video is called
        let result = is_hevc_video(Path::new("/any/video")).await;

        // THEN the error message reports "not found at" with the path
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("not found at"),
            "expected 'not found at' in: {err_str}"
        );
        assert!(
            err_str.contains("/nonexistent/ffprobe"),
            "expected path in: {err_str}"
        );
    }

    #[test]
    fn test_error_message_not_found_ffprobe_has_moov() {
        // GIVEN a nonexistent ffprobe path
        let _guard = EnvVarGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

        // WHEN has_moov_at_start is called
        let result = has_moov_at_start(Path::new("/any/video"));

        // THEN the error message reports "not found at" with the path
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("not found at"),
            "expected 'not found at' in: {err_str}"
        );
        assert!(
            err_str.contains("/nonexistent/ffprobe"),
            "expected path in: {err_str}"
        );
    }

    #[test]
    fn test_error_message_not_found_ffmpeg_fix_moov() {
        // GIVEN a nonexistent ffprobe path (has_moov_at_start is called first)
        // We need to make has_moov_at_start return Ok(false) so fix_moov_atom proceeds to ffmpeg
        // Actually, fix_moov_atom calls has_moov_at_start first, which also needs ffprobe.
        // So we test with valid ffprobe but invalid ffmpeg. Use a fake ffprobe that returns success.
        let temp_dir = TempDir::new().unwrap();
        let ffprobe_script = temp_dir.path().join("fake_ffprobe.sh");
        // Fake ffprobe outputs trace lines where moov offset > mdat offset
        std::fs::write(
            &ffprobe_script,
            "#!/usr/bin/env sh\n\
             echo \"type:'mdat' parent:'root' sz: 5000 100\" >&2\n\
             echo \"type:'moov' parent:'root' sz: 3000 6000\" >&2\n\
             exit 0\n",
        )
        .unwrap();
        make_executable(&ffprobe_script);

        let _ffprobe_guard = EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _ffmpeg_guard = EnvVarGuard::set("FFMPEG_PATH", "/nonexistent/ffmpeg");

        let temp_video = temp_dir.path().join("test.mp4");
        std::fs::write(&temp_video, b"fake-video").unwrap();

        // WHEN fix_moov_atom is called
        let result = fix_moov_atom(&temp_video);

        // THEN the error message reports "not found at" with the ffmpeg path
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("not found at"),
            "expected 'not found at' in: {err_str}"
        );
        assert!(
            err_str.contains("/nonexistent/ffmpeg"),
            "expected path in: {err_str}"
        );
    }

    #[test]
    fn test_status_tracking() {
        // Clear any existing state first
        clear_transcode_status("test_hash");

        // Test set and get
        let status = TranscodeStatus {
            state: TranscodeState::Pending,
            hash: "test_hash".to_string(),
            started_at: Some(Utc::now()),
            error: None,
            percent: None,
        };
        set_transcode_status("test_hash", status.clone());

        let retrieved = get_transcode_status("test_hash");
        assert!(retrieved.is_some(), "Status should exist after set");
        let status_ref = retrieved.as_ref().unwrap();
        assert_eq!(status_ref.hash, "test_hash");
        assert_eq!(status_ref.state, TranscodeState::Pending);

        // Test clear
        clear_transcode_status("test_hash");
        let after_clear = get_transcode_status("test_hash");
        assert!(after_clear.is_none(), "Status should not exist after clear");
    }

    #[test]
    fn test_claim_transcode_starts_and_deduplicates() {
        // GIVEN no known status for the hash
        clear_transcode_status("claim-hash-1");

        // WHEN the slot is claimed twice in a row
        let first = claim_transcode("claim-hash-1");
        let second = claim_transcode("claim-hash-1");

        // THEN only the first caller owns the slot; the second sees it in
        // progress instead of spawning a duplicate ffmpeg job (TOCTOU fix).
        assert_eq!(first, TranscodeClaim::Started);
        assert_eq!(second, TranscodeClaim::AlreadyInProgress);
        assert!(matches!(
            get_transcode_status("claim-hash-1").map(|s| s.state),
            Some(TranscodeState::InProgress)
        ));

        // And a cleared slot can be claimed again
        clear_transcode_status("claim-hash-1");
        assert_eq!(claim_transcode("claim-hash-1"), TranscodeClaim::Started);
        clear_transcode_status("claim-hash-1");
    }

    #[test]
    fn test_claim_transcode_reports_previous_failure() {
        clear_transcode_status("claim-hash-2");
        // A FRESH failure (within the retry cooldown) blocks re-claiming
        set_transcode_status(
            "claim-hash-2",
            TranscodeStatus {
                state: TranscodeState::Failed,
                hash: "claim-hash-2".to_string(),
                started_at: Some(Utc::now()),
                error: Some("boom".to_string()),
                percent: None,
            },
        );
        assert_eq!(
            claim_transcode("claim-hash-2"),
            TranscodeClaim::PreviouslyFailedOrTimedOut
        );

        set_transcode_status(
            "claim-hash-2",
            TranscodeStatus {
                state: TranscodeState::Timeout,
                hash: "claim-hash-2".to_string(),
                started_at: Some(Utc::now()),
                error: Some("timed out".to_string()),
                percent: None,
            },
        );
        assert_eq!(
            claim_transcode("claim-hash-2"),
            TranscodeClaim::PreviouslyFailedOrTimedOut
        );
        clear_transcode_status("claim-hash-2");
    }

    #[test]
    fn test_claim_transcode_retries_after_cooldown() {
        clear_transcode_status("claim-hash-3");
        // GIVEN a failure older than TRANSCODE_RETRY_COOLDOWN (transient
        // failures must heal without a server restart)
        set_transcode_status(
            "claim-hash-3",
            TranscodeStatus {
                state: TranscodeState::Failed,
                hash: "claim-hash-3".to_string(),
                started_at: Some(
                    Utc::now() - TRANSCODE_RETRY_COOLDOWN - chrono::Duration::seconds(1),
                ),
                error: Some("boom".to_string()),
                percent: None,
            },
        );

        // WHEN the slot is claimed again
        let claim = claim_transcode("claim-hash-3");

        // THEN the stale failure is superseded by a fresh start
        assert_eq!(claim, TranscodeClaim::Started);
        assert!(matches!(
            get_transcode_status("claim-hash-3").map(|s| s.state),
            Some(TranscodeState::InProgress)
        ));
        clear_transcode_status("claim-hash-3");
    }

    #[test]
    fn test_status_store_eviction_caps_length() {
        // GIVEN a map holding more entries than the cap, all settled
        let mut map = HashMap::new();
        for i in 0..(TRANSCODE_STATUS_STORE_CAP + 20) {
            map.insert(
                format!("settled-{}", i),
                TranscodeStatus {
                    state: TranscodeState::Completed,
                    hash: format!("settled-{}", i),
                    started_at: None,
                    error: None,
                    percent: None,
                },
            );
        }

        // WHEN eviction runs
        evict_transcode_statuses(&mut map);

        // THEN the map is capped
        assert_eq!(map.len(), TRANSCODE_STATUS_STORE_CAP);
    }

    #[test]
    fn test_status_store_eviction_prefers_in_progress() {
        // GIVEN a map over the cap with one in-progress entry among settled ones
        let mut map = HashMap::new();
        for i in 0..(TRANSCODE_STATUS_STORE_CAP + 20) {
            map.insert(
                format!("settled-{}", i),
                TranscodeStatus {
                    state: TranscodeState::Failed,
                    hash: format!("settled-{}", i),
                    started_at: None,
                    error: None,
                    percent: None,
                },
            );
        }
        map.insert(
            "in-flight".to_string(),
            TranscodeStatus {
                state: TranscodeState::InProgress,
                hash: "in-flight".to_string(),
                started_at: None,
                error: None,
                percent: None,
            },
        );

        // WHEN eviction runs
        evict_transcode_statuses(&mut map);

        // THEN in-progress entries survive while settled entries are evicted first
        assert_eq!(map.len(), TRANSCODE_STATUS_STORE_CAP);
        assert!(
            map.contains_key("in-flight"),
            "in-progress entries must be evicted last"
        );
    }

    #[test]
    fn test_status_store_eviction_never_evicts_in_progress() {
        // GIVEN a map over the cap holding only in-progress entries
        let mut map = HashMap::new();
        for i in 0..(TRANSCODE_STATUS_STORE_CAP + 20) {
            map.insert(
                format!("in-flight-{}", i),
                TranscodeStatus {
                    state: TranscodeState::InProgress,
                    hash: format!("in-flight-{}", i),
                    started_at: None,
                    error: None,
                    percent: None,
                },
            );
        }

        // WHEN eviction runs
        evict_transcode_statuses(&mut map);

        // THEN no in-progress entry is evicted: the cap is a soft limit and
        // in-flight polls keep their status
        assert_eq!(map.len(), TRANSCODE_STATUS_STORE_CAP + 20);
        assert!(
            map.contains_key("in-flight-0"),
            "in-progress entries must survive eviction"
        );
        assert!(
            map.contains_key(&format!("in-flight-{}", TRANSCODE_STATUS_STORE_CAP + 19)),
            "in-progress entries must survive eviction"
        );
    }
}
