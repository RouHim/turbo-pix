use crate::thumbnail_types::{CacheError, CacheResult, VideoMetadata};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
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
}

static TRANSCODE_STATUS_STORE: OnceLock<Mutex<HashMap<String, TranscodeStatus>>> = OnceLock::new();
static TRANSCODE_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

fn get_status_store() -> &'static Mutex<HashMap<String, TranscodeStatus>> {
    TRANSCODE_STATUS_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_transcode_semaphore() -> &'static Semaphore {
    TRANSCODE_SEMAPHORE.get_or_init(|| Semaphore::new(1))
}

pub async fn acquire_transcode_permit() -> CacheResult<SemaphorePermit<'static>> {
    get_transcode_semaphore().acquire().await.map_err(|e| {
        CacheError::VideoProcessingError(format!("Failed to acquire transcode permit: {}", e))
    })
}

pub fn set_transcode_status(hash: &str, status: TranscodeStatus) {
    let store = get_status_store();
    if let Ok(mut map) = store.lock() {
        map.insert(hash.to_string(), status);
    }
}

pub fn get_transcode_status(hash: &str) -> Option<TranscodeStatus> {
    let store = get_status_store();
    store.lock().ok().and_then(|map| map.get(hash).cloned())
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
        return format!("{binary_name} binary not found at '{path}'.");
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
                video_path.to_str().unwrap(),
            ])
            .output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| CacheError::VideoProcessingError(format_binary_error("ffprobe", &ffprobe_path_for_err, &e)))?;

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
                video_path.to_str().unwrap(),
                "-frames:v",
                "1",
                "-q:v",
                "5", // Lower quality (sufficient for semantic encoding, faster)
                output_path.to_str().unwrap(),
            ])
            .output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e)))?;

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
            args.push(video_path.to_str().unwrap().to_string());
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
                    .to_str()
                    .unwrap()
                    .to_string(),
            );
        }

        Command::new(ffmpeg_path).args(&args).output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e)))?;

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
                video_path.to_str().unwrap(),
            ])
            .output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| CacheError::VideoProcessingError(format_binary_error("ffprobe", &ffprobe_path_for_err, &e)))?;

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
        .args(["-v", "trace", path.to_str().unwrap()])
        .output()
        .map_err(|e| CacheError::VideoProcessingError(format_binary_error("ffprobe", &ffprobe_path, &e)))?;

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
            path.to_str().unwrap(),
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            temp_path.to_str().unwrap(),
        ])
        .output()
    .map_err(|e| CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path, &e)))?;

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

/// Transcode HEVC video to H.264 for browser compatibility
pub async fn transcode_hevc_to_h264(input_path: &Path, output_path: &Path) -> CacheResult<()> {
    transcode_hevc_to_h264_with_timeout(input_path, output_path, Duration::from_secs(300)).await
}

async fn transcode_hevc_to_h264_with_timeout(
    input_path: &Path,
    output_path: &Path,
    timeout_duration: Duration,
) -> CacheResult<()> {
    let ffmpeg_path = get_ffmpeg_path();
    transcode_hevc_to_h264_with_timeout_and_path(
        input_path,
        output_path,
        timeout_duration,
        ffmpeg_path,
    )
    .await
}

async fn transcode_hevc_to_h264_with_timeout_and_path(
    input_path: &Path,
    output_path: &Path,
    timeout_duration: Duration,
    ffmpeg_path: String,
) -> CacheResult<()> {
    let inner = async {
        let _permit = acquire_transcode_permit().await?;

        // Create output directory if it doesn't exist
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CacheError::VideoProcessingError(format!(
                    "Failed to create output directory: {}",
                    e
                ))
            })?;
        }

        // Try hardware-accelerated HEVC decoding first, fall back to software if unavailable
        // Use VAAPI (Video Acceleration API) for hardware-accelerated HEVC decoding on Linux
        let ffmpeg_path_for_err = ffmpeg_path.clone();
        let mut command = TokioCommand::new(ffmpeg_path);
        command.kill_on_drop(true).args([
            "-hwaccel",
            "auto", // Auto-detect hardware acceleration (VAAPI, NVDEC, etc.)
            "-i",
            input_path.to_str().unwrap(),
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
            output_path.to_str().unwrap(),
        ]);

        let output = command.output().await.map_err(|e| {
            CacheError::VideoProcessingError(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            log::error!("FFmpeg transcoding failed!");
            log::error!("FFmpeg stderr: {}", stderr);
            log::error!("FFmpeg stdout: {}", stdout);
            return Err(CacheError::VideoProcessingError(format!(
                "ffmpeg transcode exited with status {}. stderr: {}",
                output.status, stderr
            )));
        }

        Ok::<(), CacheError>(())
    };

    timeout(timeout_duration, inner).await.map_err(|_| {
        CacheError::VideoProcessingError(format!(
            "Transcoding timed out after {}s",
            timeout_duration.as_secs()
        ))
    })?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CacheConfig, Config};
    use crate::db::{create_in_memory_pool, Photo};
    use crate::thumbnail_generator::ThumbnailGenerator;
    use crate::thumbnail_types::{ThumbnailFormat, ThumbnailSize};
    use chrono::Utc;
    use std::io::{Error, ErrorKind};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::sleep;

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

    #[test]
    fn test_moov_detection() {
        let video_filename = "test_video.mp4";
        if !should_run_video_tests(video_filename) {
            eprintln!("Skipping MOOV detection test (prereqs missing or RUN_VIDEO_TESTS not set)");
            return;
        }

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

        let temp_dir = TempDir::new().unwrap();
        let source = project_photo_path(video_filename);
        let moov_start = temp_dir.path().join("moov_start.mp4");

        create_test_video_with_movflags(&source, &moov_start, "+faststart");

        let before = std::fs::metadata(&moov_start).unwrap().modified().unwrap();
        fix_moov_atom(&moov_start).unwrap();
        let after = std::fs::metadata(&moov_start).unwrap().modified().unwrap();

        assert_eq!(before, after);
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
            port: TEST_PORT,
            photo_paths: vec![],
            data_path,
            db_path,
            cache: CacheConfig {
                thumbnail_cache_path: cache_path.join("thumbnails").to_string_lossy().to_string(),
                max_cache_size_mb: 1024,
            },
            locale: "en".to_string(),
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

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    #[tokio::test]
    async fn test_transcode_semaphore() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let run = |active: Arc<AtomicUsize>, max_active: Arc<AtomicUsize>| async move {
            let _permit = acquire_transcode_permit().await.unwrap();
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            loop {
                let max = max_active.load(Ordering::SeqCst);
                if current <= max {
                    break;
                }
                if max_active
                    .compare_exchange(max, current, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    break;
                }
            }

            sleep(Duration::from_millis(150)).await;
            active.fetch_sub(1, Ordering::SeqCst);
        };

        let t1 = tokio::spawn(run(active.clone(), max_active.clone()));
        let t2 = tokio::spawn(run(active, max_active.clone()));
        t1.await.unwrap();
        t2.await.unwrap();

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_transcode_timeout() {
        let temp_dir = TempDir::new().unwrap();
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg_timeout.sh");
        std::fs::write(&ffmpeg_script, "#!/usr/bin/env sh\nsleep 2\nexit 0\n").unwrap();
        make_executable(&ffmpeg_script);

        let input = temp_dir.path().join("input.mp4");
        let output = temp_dir.path().join("output.mp4");
        std::fs::write(&input, b"not-a-real-video").unwrap();

        let result = transcode_hevc_to_h264_with_timeout_and_path(
            &input,
            &output,
            Duration::from_secs(1),
            ffmpeg_script.to_str().unwrap().to_string(),
        )
        .await;

        assert!(result.is_err(), "Expected timeout error");
        let error = format!("{}", result.unwrap_err());
        assert!(
            error.contains("timed out"),
            "Error should mention timeout, got: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_transcode_happy_path() {
        let temp_dir = TempDir::new().unwrap();
        let ffmpeg_script = temp_dir.path().join("fake_ffmpeg_ok.sh");
        std::fs::write(
            &ffmpeg_script,
            "#!/usr/bin/env sh\nfor last; do :; done\ntouch \"$last\"\nexit 0\n",
        )
        .unwrap();
        make_executable(&ffmpeg_script);

        let input = temp_dir.path().join("input.mp4");
        let output = temp_dir.path().join("nested/output.mp4");
        std::fs::write(&input, b"not-a-real-video").unwrap();

        let result = transcode_hevc_to_h264_with_timeout_and_path(
            &input,
            &output,
            Duration::from_secs(5),
            ffmpeg_script.to_str().unwrap().to_string(),
        )
        .await;

        assert!(
            result.is_ok(),
            "Expected transcode to succeed: {:?}",
            result
        );
        assert!(output.exists(), "Expected output file to be created");
    }

    #[test]
    fn test_transcode_status_json() {
        let status = TranscodeStatus {
            state: TranscodeState::InProgress,
            hash: "abc".to_string(),
            started_at: None,
            error: None,
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
        let result = extract_frame_at_time(Path::new("/any/video"), 1.0, Path::new("/any/out")).await;

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

        let _ffprobe_guard =
            EnvVarGuard::set("FFPROBE_PATH", ffprobe_script.to_str().unwrap());
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
}
