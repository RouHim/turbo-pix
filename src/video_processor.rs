use crate::thumbnail_types::{CacheError, CacheResult, VideoMetadata};
use crate::video_encoder::{self, HwPlan, SOFTWARE_ENCODER};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
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
    /// ffmpeg encoder that produced the artifact (`libx264` or a hardware
    /// encoder), set on `Completed`. `None` while unknown.
    pub encoder: Option<String>,
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
        encoder: None,
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

/// True when both ffmpeg and ffprobe are runnable. Test helper — production
/// startup already fails fast via `verify_ffmpeg_available`.
#[cfg(test)]
pub(crate) fn ffmpeg_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        std::process::Command::new(bin)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
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

/// Copy `input` into `output` as a faststart MP4 without re-encoding.
///
/// This does not second-guess the caller: a remux stream that just finished is
/// proof that the remux was needed. That matters for sources whose container
/// never had a moov atom to move (Matroska answers the moov probe with "nothing
/// to fix") and whose sidecar is the only way to cache the work the finished
/// run already paid for. No-op when the sidecar exists. Writes to a unique temp
/// file then atomically renames into place, so concurrent requests for the same
/// sidecar cannot interleave into a corrupt output.
pub async fn remux_to_faststart_mp4(input_path: &Path, output_path: &Path) -> CacheResult<()> {
    // Bound the remux by the remux semaphore so a burst of requests cannot
    // spawn unbounded blocking ffmpeg processes on the async runtime.
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

/// Removes crash-debris temp files left by a killed ffmpeg run. Returns the
/// number of files removed.
///
/// Two temp shapes exist, and NEITHER is reachable by the lazy per-request
/// cleanup after a restart: the deterministic whole-file temp
/// (`{hash}_{size}_{mtime}.mp4.tmp`, only cleaned when the SAME hash is
/// requested again) and the unique remux/moovfix temps
/// (`{stem}.{pid}.{seq}.tmp`, `{stem}.moovfix.{pid}.{ext}` — cleaned never).
/// Finished `*.mp4` artifacts are never touched: they only ever appear
/// through an atomic temp + rename, so existence means complete.
pub fn sweep_transcode_debris(cache_dir: &Path, photo_paths: &[PathBuf]) -> usize {
    let mut removed = 0;
    // The transcode tree only ever holds machine-generated cache files, so
    // any `*.tmp` there is debris. `*.moovfix.*` can also sit here in theory;
    // match it too for symmetry with the source dirs.
    if cache_dir.exists() {
        removed += sweep_tree(cache_dir, &|n| {
            n.ends_with(".tmp") || n.contains(".moovfix.")
        });
    }
    // Source dirs hold user files: only the unambiguous moovfix pattern is
    // debris there, never a bare `*.tmp`.
    for dir in photo_paths {
        if dir.exists() {
            removed += sweep_tree(dir, &|n| n.contains(".moovfix."));
        }
    }
    if removed > 0 {
        log::info!("Transcode sweep: removed {removed} leftover temp file(s)");
    }
    removed
}

fn sweep_tree(root: &Path, is_debris: &dyn Fn(&str) -> bool) -> usize {
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            removed += sweep_tree(&path, is_debris);
            continue;
        }
        if entry.file_name().to_str().is_some_and(is_debris) {
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
            } else {
                log::warn!("Transcode sweep: could not remove {}", path.display());
            }
        }
    }
    removed
}

/// What a whole-file conversion does to the video track.
///
/// The whole-file cache holds one artifact per source version, and the client
/// that reopens the video is the one whose stream produced it — so the artifact
/// must not be weaker than the stream it replaces: a source whose video the
/// client can already decode is copied, never re-encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileConversion {
    /// Re-encode the video to H.264 (libx264) for a source no browser decodes.
    Reencode,
    /// Copy the video track bit-for-bit and convert only the audio, for a
    /// source whose video the client plays but whose audio it cannot.
    VideoCopy,
}

/// The source's codecs as the capability record resolved them.
///
/// They travel together because they answer one question — what may be copied
/// instead of converted — and both must be `None` when unknown: an unknown
/// codec never selects a copy, and never picks a sample-entry tag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceCodecs<'a> {
    /// First video track of the source (`"hevc"`, `"h264"`, …).
    pub video: Option<&'a str>,
    /// First audio track of the source.
    pub audio: Option<&'a str>,
}

/// What a finished conversion actually did, so the caller can report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionOutcome {
    /// The ffmpeg encoder that produced the artifact, or `None` when the
    /// conversion did not encode video at all (a `VideoCopy`). `None` is a
    /// meaningful answer: the player hint stays hidden for a copy, because no
    /// GPU/CPU claim applies to a track that was passed through.
    pub encoder: Option<String>,
    /// True when a hardware attempt failed and the software encoder finished.
    pub fell_back: bool,
}

/// Write a whole-file conversion of `input` to `output`, reporting progress
/// percentage to `on_progress` as ffmpeg emits `-progress pipe:1` lines.
/// `on_progress` is called with `Some(percent)` (0..=100) whenever a progress
/// line arrives and the input duration is known, and never with a decreasing
/// value. When the duration is unknown, `on_progress(None)` signals "working,
/// no percent".
///
/// `conversion` picks the video handling and `codecs.audio` (the source's audio
/// codec when known, from the capability record or a probe) decides whether the
/// audio track is copied or converted — see [`build_conversion_args`].
/// `codecs.video` only matters for [`FileConversion::VideoCopy`], where it picks
/// the sample-entry tag.
///
/// A re-encode runs on the hardware encoder the startup probe selected when
/// there is one, and finishes in [`SOFTWARE_ENCODER`] if that attempt fails:
/// the probe vouches for the encoder, not for the device staying available.
/// The [`ConversionOutcome`] names the encoder that produced the artifact.
pub async fn convert_video_with_progress(
    input_path: &Path,
    output_path: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    on_progress: Arc<dyn Fn(Option<u8>) + Send + Sync>,
) -> CacheResult<ConversionOutcome> {
    // A copy never encodes, so it never asks for a hardware encoder.
    let plan = match conversion {
        FileConversion::Reencode => video_encoder::active(),
        FileConversion::VideoCopy => None,
    };
    convert_with_fallback(
        input_path,
        output_path,
        conversion,
        codecs,
        Duration::from_secs(transcode_timeout_secs()),
        get_ffmpeg_path(),
        Some(on_progress),
        plan,
    )
    .await
}

/// ffmpeg arguments for a whole-file conversion (see [`FileConversion`]).
///
/// `-map 0:v:0` takes the first video track and nothing else: without an
/// explicit map ffmpeg muxes everything it can into the MP4 (a second audio
/// track, an attached cover image, a subtitle track) and aborts the whole run
/// on the first stream the muxer cannot carry. `-map 0:a:0?` keeps the first
/// audio track; the trailing `?` is what makes a silent source convert instead
/// of failing with "Stream map '0:a:0' matches no streams".
///
/// Audio is copied only when the run re-encodes the video and the source
/// already carries a codec every browser decodes (AAC, MP3). Anything else —
/// AC-3, E-AC-3, DTS, PCM — becomes AAC, because an output file whose audio
/// track the client cannot decode is not playable at all, which defeats the
/// purpose of every cache artifact and of the whole-file escape hatch alike.
///
/// `codecs.video` is the source's first video track. A
/// [`FileConversion::VideoCopy`] of HEVC is tagged `hvc1` so the artifact's
/// sample entry matches the codec string the client declared and was told
/// about; every other case ignores it.
///
/// `plan` is the hardware encoder the startup probe selected, or `None` for
/// the software path. It only ever replaces the video encoder block of a
/// [`FileConversion::Reencode`] — a copy passes frames through untouched, and
/// `None` reproduces the historical libx264 invocation exactly (spec FR-009).
pub fn build_conversion_args(
    input: &Path,
    output: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    with_progress: bool,
    plan: Option<&HwPlan>,
) -> Vec<String> {
    let input_path = input.to_string_lossy().into_owned();
    let mut args: Vec<String> = ["-i", &input_path, "-map", "0:v:0", "-map", "0:a:0?"]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
    match conversion {
        FileConversion::Reencode => {
            // Hardware-accelerated decoding is worth asking for only when the
            // video is actually decoded (a copy never is). `auto` delegates
            // when the source is decodable in hardware and stays in software
            // otherwise, so the encoder decision is the one that needs probing.
            args.splice(0..0, ["-hwaccel", "auto"].iter().map(|arg| arg.to_string()));
            match plan {
                Some(plan) => {
                    // Device selection precedes the input; the upload filter and
                    // the encoder options belong to the output.
                    args.splice(0..0, plan.input_args());
                    if let Some(filter) = plan.upload_filter_args() {
                        args.extend(filter);
                    }
                    args.extend(plan.video_args());
                    log::info!("Converting with hardware encoder {}", plan.label());
                }
                None => args.extend(
                    [
                        "-c:v", "libx264", // More widely available than libopenh264
                        "-preset", "fast", // Good for real-time transcoding
                        "-crf", "23", // 18-28, lower = better quality
                    ]
                    .iter()
                    .map(|arg| arg.to_string()),
                ),
            }
        }
        // The client plays this video already: copying it keeps the artifact
        // bit-identical to the source instead of adding a generation of loss.
        FileConversion::VideoCopy => {
            args.extend(["-c:v", "copy"].iter().map(|arg| arg.to_string()));
            // The MP4 muxer tags a copied HEVC track `hev1` unless the source
            // already carried `hvc1`, but a client that declared HEVC support
            // was promised `hvc1.*`: a persistent sample entry named `hev1`
            // contradicts the declared codec string, so the decoder is never
            // set up. Only a copy can be HEVC — a re-encoded track is H.264,
            // for which the muxer rejects the tag outright.
            if codecs.video == Some("hevc") {
                args.extend(["-tag:v", "hvc1"].iter().map(|arg| arg.to_string()));
            }
        }
    }
    let audio_args: &[&str] = match (conversion, codecs.audio) {
        (FileConversion::Reencode, Some("aac") | Some("mp3")) => &["-c:a", "copy"],
        _ => &["-c:a", "aac", "-b:a", "160k", "-ac", "2"],
    };
    args.extend(audio_args.iter().map(|arg| arg.to_string()));
    args.extend(
        [
            "-movflags",
            "+faststart", // Enable streaming-friendly format
            "-y",         // Overwrite output file
            // Force the muxer explicitly: the temp output path ends in
            // `.mp4.tmp`, so ffmpeg cannot infer the format from the
            // extension and otherwise fails with "Error initializing
            // the muxer: Invalid argument".
            "-f",
            "mp4",
        ]
        .iter()
        .map(|arg| arg.to_string()),
    );
    if with_progress {
        // Stream progress to stdout for percent reporting; only requested when
        // somebody consumes it.
        args.extend(["-progress", "pipe:1"].iter().map(|arg| arg.to_string()));
    }
    args.push(output.to_string_lossy().into_owned());
    args
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

/// One conversion attempt, with its failure modes kept apart: the caller
/// retries an ordinary failure in software, but a timeout has already spent the
/// whole per-transcode budget and must not spend a second one.
enum Attempt {
    Done,
    Failed(String),
    TimedOut(String),
}

/// One conversion attempt with one encoder, writing through a temp file in the
/// output's directory and renaming it into place only on success.
///
/// The parameter list is the whole job description, threaded explicitly so an
/// attempt and its [`convert_with_fallback`] caller stay call-compatible;
/// grouping it into a struct would only move the same fields one level down.
#[allow(clippy::too_many_arguments)]
async fn convert_attempt(
    input_path: &Path,
    output_path: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    timeout_duration: Duration,
    ffmpeg_path: String,
    on_progress: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
    plan: Option<&HwPlan>,
) -> Attempt {
    // Write to a temp file in the SAME directory as the final path so the
    // completed file can be atomically renamed into place. A failed or
    // timed-out transcode must never leave a partial file at `output_path`,
    // which callers would otherwise treat as valid video.
    let temp_output_path = output_path.with_extension("mp4.tmp");
    let output_path_owned = output_path.to_path_buf();

    let inner = async {
        let _permit = match acquire_transcode_permit().await {
            Ok(permit) => permit,
            Err(e) => return Attempt::Failed(e.to_string()),
        };

        // Create output directory if it doesn't exist
        if let Some(parent) = temp_output_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Attempt::Failed(format!("Failed to create output directory: {e}"));
            }
        }

        // Probe the input duration once so progress can be expressed as a
        // percentage. A best-effort probe: if it fails, percent is unknown and
        // the client is told "working" with no number.
        let with_progress = on_progress.is_some();
        let duration = if with_progress {
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

        // `build_conversion_args` decides how the video and audio tracks are
        // handled; `-progress pipe:1` (when someone consumes it) streams
        // key=value progress lines to stdout, which we read incrementally to
        // report percent. `plan` picks the encoder: `Some` replaces the libx264
        // flags with a hardware encoder, `None` is the software path.
        let ffmpeg_path_for_err = ffmpeg_path.clone();
        let mut command = TokioCommand::new(ffmpeg_path);
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(build_conversion_args(
                input_path,
                &temp_output_path,
                conversion,
                codecs,
                with_progress,
                plan,
            ));

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Attempt::Failed(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e))
            }
        };

        // Drain stdout (progress) incrementally; collect stderr for the error
        // message so a long ffmpeg run cannot deadlock on a full pipe.
        let Some(stdout) = child.stdout.take() else {
            return Attempt::Failed("ffmpeg stdout pipe unavailable".to_string());
        };
        let Some(stderr) = child.stderr.take() else {
            return Attempt::Failed("ffmpeg stderr pipe unavailable".to_string());
        };

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
        let status = match child.wait().await {
            Ok(status) => status,
            Err(e) => {
                return Attempt::Failed(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e))
            }
        };
        let stderr = stderr_handle.await.unwrap_or_default();

        if !status.success() {
            log::error!("FFmpeg transcoding failed!");
            log::error!("FFmpeg stderr: {}", stderr);
            let _ = std::fs::remove_file(&temp_output_path);
            return Attempt::Failed(format!(
                "ffmpeg transcode exited with status {}. stderr: {}",
                status, stderr
            ));
        }

        // Move the completed temp file into place (atomic on the same filesystem).
        if let Err(e) = std::fs::rename(&temp_output_path, &output_path_owned) {
            let _ = std::fs::remove_file(&temp_output_path);
            return Attempt::Failed(format!("Failed to move transcoded video into place: {}", e));
        }

        Attempt::Done
    };

    match timeout(timeout_duration, inner).await {
        Ok(attempt) => attempt,
        Err(_) => {
            // The inner future (and with it the ffmpeg child, via kill_on_drop)
            // has been dropped; remove whatever partial output it wrote.
            let _ = std::fs::remove_file(&temp_output_path);
            Attempt::TimedOut(format!(
                "Transcoding timed out after {}s",
                timeout_duration.as_secs()
            ))
        }
    }
}

/// Progress callback shared by every attempt of one job, so the software retry
/// cannot lower the percentage the client already saw: each attempt's parser
/// counts from zero, the high-water mark does not.
#[derive(Clone, Default)]
struct ProgressHighWater {
    last: Arc<AtomicU8>,
}

impl ProgressHighWater {
    fn wrap(
        &self,
        callback: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
    ) -> Option<Arc<dyn Fn(Option<u8>) + Send + Sync>> {
        let last = Arc::clone(&self.last);
        callback.map(move |callback| {
            Arc::new(move |percent: Option<u8>| match percent {
                Some(percent) => {
                    if last.fetch_max(percent, Ordering::Relaxed) < percent {
                        callback(Some(percent));
                    }
                }
                None => callback(None),
            }) as Arc<dyn Fn(Option<u8>) + Send + Sync>
        })
    }
}

/// Convert with the hardware plan first, finishing in software when that
/// attempt fails.
///
/// The probe can prove that a hardware encoder opens and encodes, but not that
/// it will accept *this* source on *this* day: the device can be busy, gone, or
/// refusing the pixel format. The user must still get their video, so an
/// ordinary failure is retried once without the plan. A timeout is not retried
/// — the budget is already spent.
///
/// The arguments mirror [`convert_attempt`]; that shape is what the tests drive
/// with a plan and without one.
#[allow(clippy::too_many_arguments)]
async fn convert_with_fallback(
    input_path: &Path,
    output_path: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    timeout_duration: Duration,
    ffmpeg_path: String,
    on_progress: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
    plan: Option<HwPlan>,
) -> CacheResult<ConversionOutcome> {
    let progress = ProgressHighWater::default();
    // A copy passes the video track through, so there is no encoder to report:
    // `None` is what keeps the player hint hidden for it.
    let encodes_video = conversion != FileConversion::VideoCopy;
    let software_outcome = |fell_back: bool| ConversionOutcome {
        encoder: encodes_video.then(|| SOFTWARE_ENCODER.to_string()),
        fell_back,
    };

    let Some(plan) = plan else {
        return match convert_attempt(
            input_path,
            output_path,
            conversion,
            codecs,
            timeout_duration,
            ffmpeg_path,
            progress.wrap(on_progress),
            None,
        )
        .await
        {
            Attempt::Done => Ok(software_outcome(false)),
            Attempt::Failed(message) | Attempt::TimedOut(message) => {
                Err(CacheError::VideoProcessingError(message))
            }
        };
    };

    let first = convert_attempt(
        input_path,
        output_path,
        conversion,
        codecs,
        timeout_duration,
        ffmpeg_path.clone(),
        progress.wrap(on_progress.clone()),
        Some(&plan),
    )
    .await;

    match first {
        Attempt::Done => Ok(ConversionOutcome {
            encoder: encodes_video.then(|| plan.encoder().name().to_string()),
            fell_back: false,
        }),
        Attempt::TimedOut(message) => Err(CacheError::VideoProcessingError(message)),
        Attempt::Failed(message) => {
            log::warn!(
                "Hardware encoder {} failed ({}); retrying with {}",
                plan.label(),
                message,
                SOFTWARE_ENCODER
            );
            match convert_attempt(
                input_path,
                output_path,
                conversion,
                codecs,
                timeout_duration,
                ffmpeg_path,
                progress.wrap(on_progress),
                None,
            )
            .await
            {
                Attempt::Done => Ok(software_outcome(true)),
                Attempt::Failed(message) | Attempt::TimedOut(message) => {
                    Err(CacheError::VideoProcessingError(message))
                }
            }
        }
    }
}

/// The namespaces a transcode artifact can be written into, as they appear
/// beneath the transcode cache root (see [`namespace_dir`]).
const TRANSCODE_NAMESPACES: [&str; 3] = ["transcoded", "copied", "remux"];

/// Resolves the directory of one transcode namespace beneath `cache_dir`.
///
/// A cache dir whose last component already IS the namespace is the namespace
/// itself: `main.rs` defaults `TRANSCODE_CACHE_DIR` to
/// `{data_path}/cache/transcoded`, so the shipped configuration points
/// straight at the `transcoded/` directory and joining a second `transcoded`
/// names a path that never exists. Both the path builders and the two cleanup
/// sweeps below resolve through here, so the layout they write and the layout
/// they scan cannot drift apart again.
fn namespace_dir(cache_dir: &Path, ns: &str) -> PathBuf {
    if cache_dir.file_name().is_some_and(|n| n == ns) {
        cache_dir.to_path_buf()
    } else {
        cache_dir.join(ns)
    }
}

/// Get the path for a transcoded video in the cache
pub fn get_transcoded_path(cache_dir: &Path, original_hash: &str) -> PathBuf {
    namespace_dir(cache_dir, "transcoded").join(format!("{}.mp4", original_hash))
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
    namespace_dir(cache_dir, "transcoded").join(format!(
        "{}_{}_{}.mp4",
        original_hash, file_size, modified_millis
    ))
}

/// Video-copy cache path: the same version key as
/// [`get_transcoded_path_versioned`], in its own `copied/` namespace.
///
/// The two artifacts are not interchangeable: a copy keeps the SOURCE video
/// codec, so it is playable only by a client that declared that codec, while
/// the `transcoded/` H.264 + AAC re-encode plays everywhere. Sharing one
/// slot would let a capable client's audio-mode playthrough fill a copy whose
/// codec the next client just declared it cannot decode — and the artifact
/// would then keep answering `cached` for it.
pub fn get_copied_path_versioned(
    cache_dir: &Path,
    original_hash: &str,
    file_size: i64,
    modified_millis: i64,
) -> PathBuf {
    namespace_dir(cache_dir, "copied").join(format!(
        "{}_{}_{}.mp4",
        original_hash, file_size, modified_millis
    ))
}

/// Faststart remux sidecar path under `{cache_dir}/remux/`, versioned by the
/// source's content fingerprint (size + mtime millis) exactly like the
/// transcode cache. (Moved from handlers_video so all three transcode
/// namespaces have one home module.)
pub(crate) fn remux_sidecar_path(
    cache_dir: &str,
    original_hash: &str,
    file_size: i64,
    modified_millis: i64,
) -> std::path::PathBuf {
    namespace_dir(Path::new(cache_dir), "remux").join(format!(
        "{}_{}_{}.mp4",
        original_hash, file_size, modified_millis
    ))
}

/// Removes every transcode-cache file for `hash`: finished versioned
/// artifacts (`{hash}_*.mp4`) and temp leftovers (`{hash}_*.tmp`) in the
/// `transcoded/`, `copied/` and `remux/` namespaces.
///
/// `clear_for_hash` (CacheManager) only covers thumbnails — without this call
/// a deleted photo's conversions stay on disk forever. The `{hash}_` prefix
/// uses the full 64-hex path hash, so no other photo's files can match.
pub fn clear_transcode_cache_for_hash(hash: &str) {
    let cache_dir = std::env::var("TRANSCODE_CACHE_DIR")
        .unwrap_or_else(|_| "./data/cache/transcoded".to_string());
    let root = Path::new(&cache_dir);
    let prefix = format!("{hash}_");
    for ns in TRANSCODE_NAMESPACES {
        let dir = namespace_dir(root, ns);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(&prefix))
            {
                let path = entry.path();
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!(
                        "Failed to remove transcode cache file {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    }
}

/// Removes stale `{hash}_*.mp4` siblings in every transcode namespace except
/// `keep`.
///
/// Cache filenames fold in size+mtime, so an in-place edit produces a NEW
/// file rather than overwriting — without this the old version stays on disk
/// forever. The previous code only purged the artifact's own directory; the
/// other two namespaces (`copied/` vs `transcoded/`, plus `remux/`) kept
/// their stale copies.
///
/// `*.tmp` names are never removed: a temp is not a version, it is a
/// conversion IN FLIGHT, and one job's purge runs while another job of the
/// same hash may still be writing its own `{hash}_…tmp` (the remux fill
/// finishing while the whole-file conversion runs, and the reverse). Crash
/// debris is the startup sweep's job ([`sweep_transcode_debris`]).
pub(crate) fn purge_old_transcode_versions(cache_root: &Path, hash: &str, keep: &Path) {
    let keep_name = keep
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let prefix = format!("{hash}_");
    for ns in TRANSCODE_NAMESPACES {
        let dir = namespace_dir(cache_root, ns);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let is_old_version = entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(&prefix) && n != keep_name && n.ends_with(".mp4"));
            if is_old_version {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::{CacheConfig, Config};
    use crate::db::{create_in_memory_pool, Photo};
    use crate::thumbnail_generator::ThumbnailGenerator;
    use crate::thumbnail_types::{ThumbnailFormat, ThumbnailSize};
    use crate::video_encoder::HwEncoder;
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
    pub(crate) struct TestEnvLock {
        _mutex: Option<MutexGuard<'static, ()>>,
    }

    impl Drop for TestEnvLock {
        fn drop(&mut self) {
            TEST_ENV_LOCK_DEPTH.with(|depth| {
                depth.set(depth.get().saturating_sub(1));
            });
        }
    }

    pub(crate) fn acquire_test_env_lock() -> TestEnvLock {
        TEST_ENV_LOCK_DEPTH.with(|depth| {
            let current = depth.get();
            depth.set(current + 1);

            TestEnvLock {
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

    /// Sets an environment variable for the lifetime of the guard, holding the
    /// shared env lock so no other env-dependent test observes it mid-way. The
    /// original value (or absence) is restored on drop.
    pub(crate) struct TestEnvGuard {
        key: &'static str,
        original: Option<String>,
        _lock: TestEnvLock,
    }

    impl TestEnvGuard {
        pub(crate) fn set(key: &'static str, value: &str) -> Self {
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

    impl Drop for TestEnvGuard {
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

        if !ffmpeg_available() {
            eprintln!("ffmpeg or ffprobe not found in PATH; skipping video tests");
            return false;
        }

        true
    }

    const TEST_PORT: u16 = 18473;

    #[test]
    fn test_verify_ffmpeg_available_fails_not_found() {
        // GIVEN missing ffmpeg and ffprobe paths
        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", "/nonexistent/ffmpeg");
        let _ffprobe_guard = TestEnvGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

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

        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());
        let _ffprobe_guard = TestEnvGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

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
    // intended "broken" input for the faststart-remux test.
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

    /// The remux cache fill's core: a moov-at-end MP4 must come out of the
    /// `-c copy -movflags +faststart` pass with its moov at the front (that is
    /// what makes the sidecar streamable), and a sidecar that already exists
    /// must not be remuxed again.
    #[tokio::test]
    async fn remux_to_faststart_mp4_moves_moov_to_front() {
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
        remux_to_faststart_mp4(&moov_end, &out).await.unwrap();
        assert!(out.exists());
        assert!(
            has_moov_at_start(&out).unwrap(),
            "remux must move moov forward"
        );
        // Second call short-circuits on the finished sidecar: the remux must
        // not run twice for one artifact.
        let remuxed = std::fs::metadata(&out).unwrap().modified().unwrap();
        remux_to_faststart_mp4(&out, &out).await.unwrap();
        assert_eq!(
            std::fs::metadata(&out).unwrap().modified().unwrap(),
            remuxed,
            "an existing sidecar must not be rewritten"
        );
    }

    /// One field of the first video stream, as ffprobe reports it
    /// (`codec_tag_string`, `profile`, `pix_fmt` …) — the properties a decoder
    /// actually sees in the file.
    fn video_stream_field(path: &Path, entry: &str) -> String {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                &format!("stream={entry}"),
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .output()
            .expect("ffprobe must run");
        assert!(
            output.status.success(),
            "ffprobe failed on {}",
            path.display()
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// The first video stream's MP4 sample-entry tag, as the decoder sees it.
    fn video_codec_tag(path: &Path) -> String {
        video_stream_field(path, "codec_tag_string")
    }

    /// The whole-file copy is what a client that declared the source's video
    /// codec is served, so its video track must carry the sample entry that
    /// codec string names. ffmpeg's muxer would otherwise tag a copied HEVC
    /// track `hev1`, which contradicts the `hvc1.*` the client was promised.
    #[tokio::test]
    async fn video_copy_tags_a_hevc_track_hvc1_and_leaves_h264_alone() {
        let _lock = acquire_test_env_lock();
        let temp = TempDir::new().unwrap();
        let hevc_fixture = Path::new("test-data/test_video_hevc.mp4");
        let h264_fixture = Path::new("test-data/test_video_ac3.mp4");
        if !hevc_fixture.exists() || !h264_fixture.exists() || !ffmpeg_available() {
            eprintln!("skipping: fixtures or ffmpeg unavailable");
            return;
        }

        // Matroska carries no sample-entry tags, so copying this source into an
        // MP4 tags the track `hev1` by default — the shape a real untagged HEVC
        // library file has.
        let hevc_source = temp.path().join("hevc_source.mkv");
        let status = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(hevc_fixture)
            .args(["-map", "0:v:0", "-c", "copy", "-f", "matroska"])
            .arg(&hevc_source)
            .status()
            .expect("ffmpeg must run for the test source");
        assert!(status.success(), "building the HEVC source must succeed");

        let noop: Arc<dyn Fn(Option<u8>) + Send + Sync> = Arc::new(|_| {});
        let hevc_copy = temp.path().join("hevc_copy.mp4");
        convert_video_with_progress(
            &hevc_source,
            &hevc_copy,
            FileConversion::VideoCopy,
            SourceCodecs {
                video: Some("hevc"),
                audio: None,
            },
            noop.clone(),
        )
        .await
        .expect("an HEVC video copy must convert");
        // The H.264 side also proves the tag is conditional: ffmpeg rejects
        // `-tag:v hvc1` for an H.264 track, so a blanket tag would fail here.
        let h264_copy = temp.path().join("h264_copy.mp4");
        convert_video_with_progress(
            h264_fixture,
            &h264_copy,
            FileConversion::VideoCopy,
            SourceCodecs {
                video: Some("h264"),
                audio: Some("ac3"),
            },
            noop,
        )
        .await
        .expect("an H.264 video copy must convert");

        assert_eq!(
            video_codec_tag(&hevc_copy),
            "hvc1",
            "a copied HEVC track must carry the advertised hvc1 sample entry"
        );
        assert_eq!(
            video_codec_tag(&h264_copy),
            "avc1",
            "an H.264 copy keeps its own sample entry"
        );
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
            tile_url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png".to_string(),
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
    pub(crate) fn make_executable(path: &Path) {
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
        let _env = TestEnvGuard::set("TURBO_PIX_MAX_TRANSCODES", "0");
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
        let _env = TestEnvGuard::set("TURBO_PIX_MAX_TRANSCODES", "4");
        assert_eq!(transcode_max_pool(), 4);
        let _env2 = TestEnvGuard::set("TURBO_PIX_MAX_TRANSCODES", "0");
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

        let _ffprobe_guard = TestEnvGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", ffmpeg_script.to_str().unwrap());

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

        convert_video_with_progress(
            &input,
            &output,
            FileConversion::Reencode,
            SourceCodecs {
                video: None,
                audio: Some("aac"),
            },
            on_progress,
        )
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

        let result = convert_with_fallback(
            &input,
            &output,
            FileConversion::Reencode,
            SourceCodecs::default(),
            Duration::from_secs(1),
            ffmpeg_script.to_str().unwrap().to_string(),
            None,
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

        let result = convert_with_fallback(
            &input,
            &output,
            FileConversion::Reencode,
            SourceCodecs::default(),
            Duration::from_secs(5),
            ffmpeg_script.to_str().unwrap().to_string(),
            None,
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

        let result = convert_with_fallback(
            &input,
            &output,
            FileConversion::Reencode,
            SourceCodecs::default(),
            Duration::from_secs(5),
            ffmpeg_script.to_str().unwrap().to_string(),
            None,
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

    /// The whole-file artifact is what every reopened video and every
    /// escape-hatch request is served, so neither mode may produce something
    /// weaker than the stream it replaces: audio the client cannot decode is
    /// converted (copying AC-3 into an MP4 leaves a file Chromium plays
    /// silently or refuses outright), and video the client already decodes is
    /// copied rather than re-encoded.
    #[test]
    fn conversion_args_follow_the_conversion_mode() {
        let input = Path::new("/photos/source.mkv");
        let output = Path::new("/cache/out.mp4.tmp");
        let args = |conversion, video: Option<&str>, audio: Option<&str>, with_progress| {
            build_conversion_args(
                input,
                output,
                conversion,
                SourceCodecs { video, audio },
                with_progress,
                None,
            )
            .join(" ")
        };

        let aac = args(FileConversion::Reencode, Some("hevc"), Some("aac"), false);
        assert!(aac.contains("-c:v libx264 -preset fast -crf 23"), "{aac}");
        assert!(aac.contains("-c:a copy"), "AAC must be copied: {aac}");
        assert!(aac.contains("-hwaccel auto"), "{aac}");
        assert!(
            aac.contains("-map 0:v:0 -map 0:a:0?"),
            "only the first video and audio track are mapped: {aac}"
        );
        assert!(
            aac.contains("-movflags +faststart") && aac.contains("-f mp4"),
            "the artifact stays a faststart MP4: {aac}"
        );
        assert!(
            !aac.contains("-progress"),
            "no progress pipe is opened when nobody consumes it: {aac}"
        );

        let mp3 = args(FileConversion::Reencode, Some("h264"), Some("mp3"), false);
        assert!(
            mp3.contains("-c:a copy"),
            "MP3 is legal MP4 audio and must be copied: {mp3}"
        );

        let ac3 = args(FileConversion::Reencode, Some("h264"), Some("ac3"), true);
        assert!(
            ac3.contains("-c:a aac -b:a 160k -ac 2"),
            "AC-3 must be converted to AAC: {ac3}"
        );
        assert!(
            ac3.contains("-map 0:a:0?"),
            "the converted audio still needs mapping: {ac3}"
        );
        assert!(
            ac3.contains("-progress pipe:1"),
            "progress is reported when the caller asks for it: {ac3}"
        );

        let unknown = args(FileConversion::Reencode, None, None, false);
        assert!(
            unknown.contains("-c:a aac"),
            "an unknown source audio codec must be converted, never copied: {unknown}"
        );

        // A source whose video the client already plays keeps its video track
        // bit-for-bit — re-encoding it would add a generation of loss to every
        // later open — and only pays for its undecodable audio.
        let copy = args(FileConversion::VideoCopy, Some("h264"), Some("ac3"), false);
        assert!(copy.contains("-c:v copy"), "video must be copied: {copy}");
        assert!(
            !copy.contains("libx264"),
            "video must not be re-encoded: {copy}"
        );
        assert!(
            copy.contains("-c:a aac -b:a 160k -ac 2"),
            "the undecodable audio still becomes AAC: {copy}"
        );
        assert!(
            copy.contains("-map 0:v:0 -map 0:a:0?"),
            "the copied video still needs mapping: {copy}"
        );
        assert!(
            !copy.contains("-hwaccel"),
            "a copy decodes nothing, so no hardware acceleration is requested: {copy}"
        );
        assert!(
            !copy.contains("-tag:v"),
            "an H.264 copy keeps its avc1 sample entry: {copy}"
        );

        // A copied HEVC track must carry the `hvc1` sample entry the client was
        // promised: the muxer's `hev1` default contradicts the declared codec
        // string and the decoder is never set up.
        let hevc_copy = args(FileConversion::VideoCopy, Some("hevc"), Some("aac"), false);
        assert!(
            hevc_copy.contains("-c:v copy -tag:v hvc1"),
            "a copied HEVC track must be tagged hvc1: {hevc_copy}"
        );
        // …while a re-encode to H.264 must not: the muxer rejects the tag for
        // anything but HEVC and fails the whole conversion.
        let hevc_reencode = args(FileConversion::Reencode, Some("hevc"), Some("aac"), false);
        assert!(
            !hevc_reencode.contains("-tag:v"),
            "a re-encode emits H.264 and must stay avc1: {hevc_reencode}"
        );
    }

    /// A copy never encodes, and the software path is what a GPU-less host
    /// runs, so a planless build has to produce the historical invocation
    /// byte-for-byte (spec FR-009).
    #[test]
    fn reencode_args_are_unchanged_without_a_plan() {
        // GIVEN the software path (no plan, exactly what a GPU-less host uses)
        let args = build_conversion_args(
            Path::new("/in.mp4"),
            Path::new("/out.mp4"),
            FileConversion::Reencode,
            SourceCodecs {
                video: Some("hevc"),
                audio: Some("aac"),
            },
            false,
            None,
        )
        .join(" ");

        // THEN the ffmpeg invocation is byte-for-byte the historical one
        assert_eq!(
            args,
            "-hwaccel auto -i /in.mp4 -map 0:v:0 -map 0:a:0? -c:v libx264 -preset fast -crf 23 \
             -c:a copy -movflags +faststart -y -f mp4 /out.mp4",
        );
    }

    #[test]
    fn reencode_args_swap_in_the_hardware_encoder() {
        // GIVEN a VAAPI plan on a render node
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));

        // WHEN the conversion arguments are built
        let args = build_conversion_args(
            Path::new("/in.mp4"),
            Path::new("/out.mp4"),
            FileConversion::Reencode,
            SourceCodecs {
                video: Some("hevc"),
                audio: Some("aac"),
            },
            false,
            Some(&plan),
        )
        .join(" ");

        // THEN the device precedes the input, frames are uploaded, the encoder
        // is VAAPI's and no libx264 flag survives
        let device_index = args.find("-vaapi_device").expect("device missing");
        let input_index = args.find("-i /in.mp4").expect("input missing");
        assert!(device_index < input_index, "device must precede -i: {args}");
        assert!(args.contains("-vf format=nv12,hwupload"), "{args}");
        assert!(args.contains("-c:v h264_vaapi"), "{args}");
        assert!(args.contains("-qp 23"), "{args}");
        assert!(!args.contains("libx264"), "{args}");
        assert!(!args.contains("-crf"), "{args}");
        // AND the container and audio handling are untouched
        assert!(args.contains("-movflags +faststart"), "{args}");
        assert!(args.contains("-f mp4"), "{args}");
    }

    #[test]
    fn video_copy_args_ignore_the_plan() {
        // GIVEN a plan and a conversion that only remuxes the video track
        let plan = HwPlan::new(HwEncoder::Nvenc, None);

        // WHEN the arguments are built
        let args = build_conversion_args(
            Path::new("/in.mp4"),
            Path::new("/out.mp4"),
            FileConversion::VideoCopy,
            SourceCodecs {
                video: Some("hevc"),
                audio: Some("ac3"),
            },
            false,
            Some(&plan),
        )
        .join(" ");

        // THEN nothing about the encoder changes: a copy never encodes
        assert!(args.contains("-c:v copy"), "{args}");
        assert!(!args.contains("h264_nvenc"), "{args}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hardware_failure_falls_back_to_the_software_encoder() {
        // GIVEN an ffmpeg whose hardware encoder refuses every source
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            format!(
                "#!/usr/bin/env sh\n\
                 printf '%s\\n' \"$*\" >> '{}'\n\
                 for last; do :; done\n\
                 case \"$*\" in\n\
                 *h264_vaapi*) exit 1 ;;\n\
                 esac\n\
                 touch \"$last\"\n\
                 exit 0\n",
                log.display()
            ),
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs
        let outcome = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs {
                video: Some("hevc"),
                audio: None,
            },
            Duration::from_secs(30),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await
        .expect("the software retry must finish the job");

        // THEN the artifact exists, the outcome names the software encoder, and
        // both attempts were made against the same output
        assert!(output.exists(), "the fallback artifact must exist");
        assert_eq!(outcome.encoder.as_deref(), Some("libx264"));
        assert!(outcome.fell_back);
        let lines = std::fs::read_to_string(&log).unwrap();
        assert!(
            lines.lines().any(|line| line.contains("h264_vaapi")),
            "{lines}"
        );
        assert!(
            lines.lines().any(|line| line.contains("libx264")),
            "{lines}"
        );
        // AND no temp file is left behind
        assert!(!output.with_extension("mp4.tmp").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_successful_hardware_conversion_reports_the_hardware_encoder() {
        // GIVEN an ffmpeg whose hardware encoder works
        let temp = TempDir::new().unwrap();
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            "#!/usr/bin/env sh\nfor last; do :; done\ntouch \"$last\"\nexit 0\n",
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs
        let outcome = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs {
                video: Some("hevc"),
                audio: None,
            },
            Duration::from_secs(30),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await
        .expect("the hardware conversion must succeed");

        // THEN the outcome names it and reports no fallback
        assert_eq!(outcome.encoder.as_deref(), Some("h264_vaapi"));
        assert!(!outcome.fell_back);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_video_copy_reports_no_encoder() {
        // GIVEN a conversion that copies the video track (the `copied/`
        // namespace, used for audio-only conversions)
        let temp = TempDir::new().unwrap();
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            "#!/usr/bin/env sh\nfor last; do :; done\ntouch \"$last\"\nexit 0\n",
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs
        let outcome = convert_with_fallback(
            Path::new("test-data/test_video_ac3.mp4"),
            &output,
            FileConversion::VideoCopy,
            SourceCodecs {
                video: Some("h264"),
                audio: Some("ac3"),
            },
            Duration::from_secs(30),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            None,
        )
        .await
        .expect("the copy must succeed");

        // THEN there is no encoder to report: nothing encoded the video, so the
        // player hint stays hidden instead of claiming a CPU conversion
        assert_eq!(outcome.encoder, None);
        assert!(!outcome.fell_back);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_fallback_reports_the_software_error_and_leaves_no_debris() {
        // GIVEN an ffmpeg that fails for every encoder
        let temp = TempDir::new().unwrap();
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            "#!/usr/bin/env sh\nprintf '%s\\n' 'boom' >&2\nexit 1\n",
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs
        let result = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs {
                video: Some("hevc"),
                audio: None,
            },
            Duration::from_secs(30),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await;

        // THEN the failure surfaces and nothing is left at the output path
        assert!(result.is_err(), "both attempts failed, so the job fails");
        assert!(!output.exists(), "no partial artifact may survive");
        assert!(!output.with_extension("mp4.tmp").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_hardware_attempt_is_not_retried_in_software() {
        // GIVEN an ffmpeg whose hardware attempt hangs past the deadline
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            format!(
                "#!/usr/bin/env sh\n\
                 printf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$*\" in\n\
                 *h264_vaapi*) sleep 30 ;;\n\
                 esac\n\
                 for last; do :; done\n\
                 touch \"$last\"\n",
                log.display()
            ),
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs out of time
        let result = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs {
                video: Some("hevc"),
                audio: None,
            },
            Duration::from_secs(1),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await;

        // THEN it fails without spending a second budget: the hardware attempt
        // already consumed the whole per-transcode deadline
        assert!(result.is_err());
        let lines = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            lines.lines().filter(|line| !line.is_empty()).count(),
            1,
            "a timeout must not trigger a second attempt: {lines}"
        );
    }

    #[test]
    fn progress_never_goes_backwards_across_a_fallback() {
        // GIVEN a high-water wrapper shared by two attempts
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = Arc::clone(&seen);
        let callback: Arc<dyn Fn(Option<u8>) + Send + Sync> =
            Arc::new(move |percent| seen_clone.lock().unwrap().push(percent));
        let wrapper = ProgressHighWater::default().wrap(Some(callback));

        // WHEN the hardware attempt reports 30 and the software retry restarts
        // from zero and climbs to 40
        for percent in [Some(30), Some(10), Some(20), Some(40)] {
            wrapper.as_ref().unwrap()(percent);
        }

        // THEN only the forward-moving values reach the client
        assert_eq!(*seen.lock().unwrap(), vec![Some(30), Some(40)]);
    }

    #[test]
    fn test_transcode_status_json() {
        let status = TranscodeStatus {
            state: TranscodeState::InProgress,
            hash: "abc".to_string(),
            started_at: None,
            error: None,
            percent: None,
            encoder: Some("h264_vaapi".to_string()),
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
        assert!(
            json.contains("\"encoder\":\"h264_vaapi\""),
            "JSON should carry the encoder that produced the artifact, got: {}",
            json
        );
    }

    #[tokio::test]
    async fn test_error_message_not_found_ffprobe_extract_metadata() {
        // GIVEN a nonexistent ffprobe path
        let _guard = TestEnvGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

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
        let _guard = TestEnvGuard::set("FFMPEG_PATH", "/nonexistent/ffmpeg");

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
        let _guard = TestEnvGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

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
        let _guard = TestEnvGuard::set("FFPROBE_PATH", "/nonexistent/ffprobe");

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

        let _ffprobe_guard = TestEnvGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", "/nonexistent/ffmpeg");

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
            encoder: None,
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
                encoder: None,
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
                encoder: None,
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
                encoder: None,
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
                    encoder: None,
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
                    encoder: None,
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
                encoder: None,
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
                    encoder: None,
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
    #[test]
    fn sweep_transcode_debris_removes_temps_and_keeps_finished_artifacts() {
        // GIVEN a cache tree with crash leftovers, finished artifacts, and a photo dir with moovfix debris
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        for ns in ["transcoded", "copied", "remux"] {
            std::fs::create_dir_all(root.join(ns)).unwrap();
        }
        let hash = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let other = "1111111111111111111111111111111111111111111111111111111111111111";
        let finished = root.join("transcoded").join(format!("{hash}_100_200.mp4"));
        std::fs::write(&finished, b"done").unwrap();
        // deterministic whole-file temp from a killed transcode
        let whole_tmp = root.join("copied").join(format!("{hash}_100_200.mp4.tmp"));
        std::fs::write(&whole_tmp, b"partial").unwrap();
        // unique remux temp from a killed remux
        let remux_tmp = root
            .join("remux")
            .join(format!("{hash}_100_200.12345.0.tmp"));
        std::fs::write(&remux_tmp, b"partial").unwrap();
        // another hash's finished file must survive
        let other_finished = root.join("transcoded").join(format!("{other}_100_200.mp4"));
        std::fs::write(&other_finished, b"done").unwrap();
        // moovfix debris next to a source file
        let photos = root.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        let moovfix = photos.join("clip.moovfix.12345.mp4");
        std::fs::write(&moovfix, b"partial").unwrap();
        let real = photos.join("clip.mp4");
        std::fs::write(&real, b"video").unwrap();

        // WHEN the sweep runs
        let removed = sweep_transcode_debris(root, std::slice::from_ref(&photos));

        // THEN exactly the three debris files are gone, everything else survives
        assert_eq!(removed, 3);
        assert!(!whole_tmp.exists());
        assert!(!remux_tmp.exists());
        assert!(!moovfix.exists());
        assert!(finished.exists());
        assert!(other_finished.exists());
        assert!(real.exists());
    }
    #[tokio::test]
    async fn clear_transcode_cache_for_hash_removes_all_namespaces() {
        // GIVEN versioned artifacts plus a temp in every namespace, and another hash's files
        let _lock = acquire_test_env_lock();
        let temp = TempDir::new().unwrap();
        let _env = TestEnvGuard::set("TRANSCODE_CACHE_DIR", temp.path().to_str().unwrap());
        let hash = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let other = "2222222222222222222222222222222222222222222222222222222222222222";
        let mut doomed = Vec::new();
        for ns in ["transcoded", "copied", "remux"] {
            let dir = temp.path().join(ns);
            std::fs::create_dir_all(&dir).unwrap();
            for name in [
                format!("{hash}_100_200.mp4"),
                format!("{hash}_300_400.mp4"),
                format!("{hash}_100_200.mp4.tmp"),
            ] {
                let p = dir.join(name);
                std::fs::write(&p, b"x").unwrap();
                doomed.push(p);
            }
        }
        let kept = temp
            .path()
            .join("transcoded")
            .join(format!("{other}_100_200.mp4"));
        std::fs::write(&kept, b"x").unwrap();

        // WHEN the hash's transcode cache is cleared
        clear_transcode_cache_for_hash(hash);

        // THEN every `{hash}_*` file is gone in all three namespaces, the other hash survives
        assert!(doomed.iter().all(|p| !p.exists()), "{doomed:?}");
        assert!(kept.exists());
    }

    /// The shipped layout: `main.rs` defaults `TRANSCODE_CACHE_DIR` to
    /// `{data_path}/cache/transcoded`, so the cache root IS the `transcoded/`
    /// namespace and the re-encode sits directly in it. A cleanup that joins
    /// another `transcoded` scans a directory that never exists and silently
    /// keeps the largest cache file of every deleted photo — which the
    /// parent-rooted test above cannot see, because there the join is right.
    #[tokio::test]
    async fn clear_transcode_cache_for_hash_scans_the_production_cache_root() {
        // GIVEN `$TRANSCODE_CACHE_DIR` pointing at the `transcoded/` directory
        // itself, with `copied/` and `remux/` below it, and another hash's file
        let _lock = acquire_test_env_lock();
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("transcoded");
        std::fs::create_dir_all(&root).unwrap();
        let _env = TestEnvGuard::set("TRANSCODE_CACHE_DIR", root.to_str().unwrap());
        let hash = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let other = "2222222222222222222222222222222222222222222222222222222222222222";
        let mut doomed = vec![
            root.join(format!("{hash}_100_200.mp4")),
            root.join(format!("{hash}_100_200.mp4.tmp")),
        ];
        for ns in ["copied", "remux"] {
            let dir = root.join(ns);
            std::fs::create_dir_all(&dir).unwrap();
            doomed.push(dir.join(format!("{hash}_100_200.mp4")));
        }
        doomed.iter().for_each(|p| std::fs::write(p, b"x").unwrap());
        let kept = root.join(format!("{other}_100_200.mp4"));
        std::fs::write(&kept, b"x").unwrap();

        // WHEN the hash's transcode cache is cleared
        clear_transcode_cache_for_hash(hash);

        // THEN the re-encode in the cache root itself and the two siblings are
        // gone, and the other hash survives
        assert!(doomed.iter().all(|p| !p.exists()), "{doomed:?}");
        assert!(kept.exists());
    }

    #[test]
    fn purge_old_transcode_versions_keeps_only_new_artifact() {
        // GIVEN old versions across all namespaces plus in-flight temps and another hash's file
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let hash = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let other = "3333333333333333333333333333333333333333333333333333333333333333";
        let new_path = root.join("transcoded").join(format!("{hash}_300_400.mp4"));
        let mut doomed = Vec::new();
        let mut in_flight = Vec::new();
        for (ns, stale, temps) in [
            (
                "transcoded",
                vec![format!("{hash}_100_200.mp4")],
                vec![format!("{hash}_100_200.mp4.tmp")],
            ),
            ("copied", vec![format!("{hash}_100_200.mp4")], Vec::new()),
            (
                "remux",
                vec![format!("{hash}_100_200.mp4")],
                vec![format!("{hash}_100_200.9.0.tmp")],
            ),
        ] {
            let dir = root.join(ns);
            std::fs::create_dir_all(&dir).unwrap();
            for name in stale {
                let p = dir.join(name);
                std::fs::write(&p, b"x").unwrap();
                doomed.push(p);
            }
            for name in temps {
                let p = dir.join(name);
                std::fs::write(&p, b"x").unwrap();
                in_flight.push(p);
            }
        }
        std::fs::create_dir_all(root.join("transcoded")).unwrap();
        std::fs::write(&new_path, b"new").unwrap();
        let kept = root.join("copied").join(format!("{other}_100_200.mp4"));
        std::fs::write(&kept, b"x").unwrap();

        // WHEN old versions are purged keeping the new artifact
        purge_old_transcode_versions(root, hash, &new_path);

        // THEN only the new artifact and the other hash survive, and the temps
        // are left alone: a temp is a conversion in flight (the remux fill of
        // this very hash can be mid-write), not a stale version
        assert!(doomed.iter().all(|p| !p.exists()), "{doomed:?}");
        assert!(in_flight.iter().all(|p| p.exists()), "{in_flight:?}");
        assert!(new_path.exists());
        assert!(kept.exists());
    }

    /// The shipped layout (see the `clear` twin above): with the cache root
    /// being the `transcoded/` namespace itself, a sweep that joins another
    /// `transcoded` finds nothing and the "one version file per hash"
    /// invariant breaks after the first in-place edit.
    #[test]
    fn purge_old_transcode_versions_scans_the_production_cache_root() {
        // GIVEN `$TRANSCODE_CACHE_DIR` itself as the transcoded namespace, with
        // `copied/` and `remux/` below it, plus another hash's file
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("transcoded");
        std::fs::create_dir_all(&root).unwrap();
        let hash = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let other = "4444444444444444444444444444444444444444444444444444444444444444";
        let new_path = root.join(format!("{hash}_300_400.mp4"));
        std::fs::write(&new_path, b"new").unwrap();
        let mut doomed = vec![root.join(format!("{hash}_100_200.mp4"))];
        for ns in ["copied", "remux"] {
            let dir = root.join(ns);
            std::fs::create_dir_all(&dir).unwrap();
            doomed.push(dir.join(format!("{hash}_100_200.mp4")));
        }
        doomed.iter().for_each(|p| std::fs::write(p, b"x").unwrap());
        let kept = root.join("copied").join(format!("{other}_100_200.mp4"));
        std::fs::write(&kept, b"x").unwrap();

        // WHEN old versions are purged keeping the new artifact
        purge_old_transcode_versions(&root, hash, &new_path);

        // THEN the superseded version in the cache root itself and the stale
        // copies in both siblings are gone, and the other hash survives
        assert!(doomed.iter().all(|p| !p.exists()), "{doomed:?}");
        assert!(new_path.exists());
        assert!(kept.exists());
    }
}
