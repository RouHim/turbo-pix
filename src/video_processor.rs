use crate::thumbnail_types::{CacheError, CacheResult, VideoMetadata};
use std::path::{Path, PathBuf};
use std::process::Command;

fn get_ffmpeg_path() -> String {
    std::env::var("FFMPEG_PATH").unwrap_or_else(|_| "ffmpeg".to_string())
}

fn get_ffprobe_path() -> String {
    std::env::var("FFPROBE_PATH").unwrap_or_else(|_| "ffprobe".to_string())
}

pub async fn extract_video_metadata(video_path: &Path) -> CacheResult<VideoMetadata> {
    let video_path = video_path.to_path_buf();
    let ffprobe_path = get_ffprobe_path();

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
    .map_err(|e| CacheError::VideoProcessingError(format!("ffprobe failed: {}", e)))?;

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
    .map_err(|e| CacheError::VideoProcessingError(format!("ffmpeg failed: {}", e)))?;

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
    let frame_times = frame_times.to_vec();
    let frame_count = frame_times.len();

    let output = tokio::task::spawn_blocking(move || {
        // Build filter_complex for extracting frames at specific times
        // select='eq(t,10)+eq(t,20)+eq(t,30)' selects frames at 10s, 20s, 30s
        let select_expr = frame_times
            .iter()
            .map(|t| format!("eq(t\\,{:.2})", t))
            .collect::<Vec<_>>()
            .join("+");

        let filter_complex = format!(
            "[0:v]select='{}',split={}{}",
            select_expr,
            frame_times.len(),
            (0..frame_times.len())
                .map(|i| format!("[f{}]", i))
                .collect::<Vec<_>>()
                .join("")
        );

        // Build output arguments: -map [f0] out0.jpg -map [f1] out1.jpg ...
        let mut args = vec![
            "-y".to_string(),
            "-i".to_string(),
            video_path.to_str().unwrap().to_string(),
            "-filter_complex".to_string(),
            filter_complex,
        ];

        for i in 0..frame_times.len() {
            args.push("-map".to_string());
            args.push(format!("[f{}]", i));
            args.push("-q:v".to_string());
            args.push("5".to_string()); // Lower quality for semantic encoding
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
    .map_err(|e| CacheError::VideoProcessingError(format!("ffmpeg batch failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CacheError::VideoProcessingError(format!(
            "ffmpeg batch extraction failed: {}. stderr: {}",
            output.status, stderr
        )));
    }

    // Return paths to extracted frames
    Ok((0..frame_count)
        .map(|i| output_dir_clone.join(format!("frame_{}.jpg", i)))
        .collect())
}

/// Check if a video uses HEVC codec
pub async fn is_hevc_video(video_path: &Path) -> CacheResult<bool> {
    let video_path = video_path.to_path_buf();
    let ffprobe_path = get_ffprobe_path();

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
    .map_err(|e| CacheError::VideoProcessingError(format!("ffprobe failed: {}", e)))?;

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

/// Transcode HEVC video to H.264 for browser compatibility
pub async fn transcode_hevc_to_h264(input_path: &Path, output_path: &Path) -> CacheResult<()> {
    // Create output directory if it doesn't exist
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CacheError::VideoProcessingError(format!("Failed to create output directory: {}", e))
        })?;
    }

    let input_path = input_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let ffmpeg_path = get_ffmpeg_path();

    // Try hardware-accelerated HEVC decoding first, fall back to software if unavailable
    // Use VAAPI (Video Acceleration API) for hardware-accelerated HEVC decoding on Linux
    let output = tokio::task::spawn_blocking(move || {
        Command::new(ffmpeg_path)
            .args([
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
            ])
            .output()
    })
    .await
    .map_err(|e| CacheError::IoError(std::io::Error::other(e)))?
    .map_err(|e| CacheError::VideoProcessingError(format!("ffmpeg transcode failed: {}", e)))?;

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

    Ok(())
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
    use tempfile::TempDir;

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
            collage: crate::config::CollageConfig::default(),
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
        let db_pool = create_in_memory_pool().unwrap();
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
        let db_pool = create_in_memory_pool().unwrap();
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
}
