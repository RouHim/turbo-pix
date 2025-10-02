pub use crate::cache_manager::CacheManager;
pub use crate::thumbnail_generator::ThumbnailGenerator;
pub use crate::thumbnail_types::ThumbnailSize;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CacheConfig, Config};
    use crate::db::{create_in_memory_pool, Photo};
    use chrono::Utc;
    use tempfile::TempDir;

    mod thumbnail_tests {
        use super::*;
        use crate::thumbnail_types::{CacheError, CacheKey, VideoMetadata};

        // Helper: project-local path to test-data/<filename>
        fn project_photo_path(filename: &str) -> std::path::PathBuf {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("test-data")
                .join(filename)
        }

        // Helper: returns true if command exists (via -version)
        fn has_command(cmd: &str) -> bool {
            std::process::Command::new(cmd)
                .arg("-version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }

        // Guard: require RUN_VIDEO_TESTS and ffmpeg/ffprobe and the sample file
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
                port: 8080,
                photo_paths: vec![],
                data_path,
                db_path,
                cache: CacheConfig {
                    thumbnail_cache_path: cache_path
                        .join("thumbnails")
                        .to_string_lossy()
                        .to_string(),
                },
            };

            (config, temp_dir)
        }

        fn create_test_image(path: &std::path::Path) -> std::io::Result<()> {
            use image::{ImageBuffer, Rgb};

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Create a simple 10x10 red image
            let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(10, 10, |_x, _y| {
                Rgb([255, 0, 0]) // Red pixel
            });

            img.save(path)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            Ok(())
        }

        fn create_test_photo(path: &str) -> Photo {
            let now = Utc::now();
            Photo {
                hash_sha256: "a".repeat(64),
                file_path: path.to_string(),
                filename: "test.jpg".to_string(),
                file_size: 1024,
                mime_type: Some("image/jpeg".to_string()),
                taken_at: Some(now),
                date_modified: now,
                date_indexed: Some(now),
                camera_make: Some("Test Camera".to_string()),
                camera_model: Some("Test Model".to_string()),
                lens_make: None,
                lens_model: None,
                iso: Some(100),
                aperture: Some(2.8),
                shutter_speed: Some("1/100".to_string()),
                focal_length: Some(50.0),
                width: Some(100),
                height: Some(100),
                color_space: Some("sRGB".to_string()),
                white_balance: Some("Auto".to_string()),
                exposure_mode: Some("Auto".to_string()),
                metering_mode: Some("Center-weighted".to_string()),
                orientation: Some(1),
                flash_used: Some(false),
                latitude: None,
                longitude: None,
                location_name: None,

                thumbnail_path: None,
                has_thumbnail: Some(false),
                country: None,
                keywords: None,
                faces_detected: None,
                objects_detected: None,
                colors: None,
                duration: None,
                video_codec: None,
                audio_codec: None,
                bitrate: None,
                frame_rate: None,
                is_favorite: Some(false),
                created_at: now,
                updated_at: now,
            }
        }

        #[tokio::test]
        async fn test_thumbnail_generator_creation() {
            let (config, _temp_dir) = create_test_config();
            let db_pool = create_in_memory_pool().unwrap();
            let _generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

            // Cache directory should be created
            assert!(config
                .cache
                .thumbnail_cache_path
                .parse::<std::path::PathBuf>()
                .unwrap()
                .exists());
        }

        #[tokio::test]
        async fn test_thumbnail_generation() {
            let (config, temp_dir) = create_test_config();
            let db_pool = create_in_memory_pool().unwrap();
            let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

            // Create test image
            let image_path = temp_dir.path().join("test.jpg");
            create_test_image(&image_path).unwrap();

            let photo = create_test_photo(&image_path.to_string_lossy());

            // Generate thumbnail
            let result = generator
                .get_or_generate(&photo, ThumbnailSize::Small)
                .await;
            assert!(result.is_ok());

            let thumbnail_data = result.unwrap();
            assert!(!thumbnail_data.is_empty());

            // Should be cached on disk now
            let cache_key = CacheKey::from_photo(&photo, ThumbnailSize::Small).unwrap();
            let cache_path = generator.get_cache_path(&cache_key);
            assert!(cache_path.exists());
        }

        #[tokio::test]
        async fn test_thumbnail_cache_hit() {
            let (config, temp_dir) = create_test_config();
            let db_pool = create_in_memory_pool().unwrap();
            let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

            // Create test image
            let image_path = temp_dir.path().join("test.jpg");
            create_test_image(&image_path).unwrap();

            let photo = create_test_photo(&image_path.to_string_lossy());

            // First call - cache miss, generates thumbnail
            let result1 = generator
                .get_or_generate(&photo, ThumbnailSize::Medium)
                .await
                .unwrap();

            // Second call - cache hit, returns cached version
            let result2 = generator
                .get_or_generate(&photo, ThumbnailSize::Medium)
                .await
                .unwrap();

            assert_eq!(result1, result2);
        }

        #[tokio::test]
        async fn test_thumbnail_different_sizes() {
            let (config, temp_dir) = create_test_config();
            let db_pool = create_in_memory_pool().unwrap();
            let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

            // Create test image
            let image_path = temp_dir.path().join("test.jpg");
            create_test_image(&image_path).unwrap();

            let photo = create_test_photo(&image_path.to_string_lossy());

            // Generate different sizes
            let small = generator
                .get_or_generate(&photo, ThumbnailSize::Small)
                .await
                .unwrap();
            let medium = generator
                .get_or_generate(&photo, ThumbnailSize::Medium)
                .await
                .unwrap();
            let large = generator
                .get_or_generate(&photo, ThumbnailSize::Large)
                .await
                .unwrap();

            // All should succeed and be different
            assert!(!small.is_empty());
            assert!(!medium.is_empty());
            assert!(!large.is_empty());

            // Verify cache files exist for each size
            let small_key = CacheKey::from_photo(&photo, ThumbnailSize::Small).unwrap();
            let medium_key = CacheKey::from_photo(&photo, ThumbnailSize::Medium).unwrap();
            let large_key = CacheKey::from_photo(&photo, ThumbnailSize::Large).unwrap();

            assert!(generator.get_cache_path(&small_key).exists());
            assert!(generator.get_cache_path(&medium_key).exists());
            assert!(generator.get_cache_path(&large_key).exists());
        }

        #[tokio::test]
        async fn test_thumbnail_nonexistent_photo() {
            let (config, _temp_dir) = create_test_config();
            let db_pool = create_in_memory_pool().unwrap();
            let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

            let photo = create_test_photo("/nonexistent/path.jpg");

            let result = generator
                .get_or_generate(&photo, ThumbnailSize::Small)
                .await;
            assert!(matches!(result, Err(CacheError::PhotoNotFound)));
        }

        #[tokio::test]
        async fn test_cache_clear() {
            let (config, temp_dir) = create_test_config();
            let db_pool = create_in_memory_pool().unwrap();
            let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

            // Create test image and generate thumbnail
            let image_path = temp_dir.path().join("test.jpg");
            create_test_image(&image_path).unwrap();

            let photo = create_test_photo(&image_path.to_string_lossy());
            generator
                .get_or_generate(&photo, ThumbnailSize::Small)
                .await
                .unwrap();

            // Verify cache file exists
            let cache_key = CacheKey::from_photo(&photo, ThumbnailSize::Small).unwrap();
            let cache_path = generator.get_cache_path(&cache_key);
            assert!(cache_path.exists());

            // Clear cache
            generator.clear_cache().await.unwrap();

            // Cache file should be gone
            assert!(!cache_path.exists());

            // Cache directory should be recreated
            assert!(std::path::PathBuf::from(&config.cache.thumbnail_cache_path).exists());
        }

        #[tokio::test]
        async fn test_cache_stats() {
            let (config, temp_dir) = create_test_config();
            let db_pool = create_in_memory_pool().unwrap();
            let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

            // Initially empty
            let (files, size) = generator.get_cache_stats().await;
            assert_eq!(files, 0);
            assert_eq!(size, 0);

            // Create test image and generate thumbnails
            let image_path = temp_dir.path().join("test.jpg");
            create_test_image(&image_path).unwrap();

            let photo = create_test_photo(&image_path.to_string_lossy());
            generator
                .get_or_generate(&photo, ThumbnailSize::Small)
                .await
                .unwrap();
            generator
                .get_or_generate(&photo, ThumbnailSize::Medium)
                .await
                .unwrap();

            // Should have 2 files with some size
            let (files, size) = generator.get_cache_stats().await;
            assert_eq!(files, 2);
            assert!(size > 0);
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
                date_modified: now,
                date_indexed: Some(now),
                camera_make: None,
                camera_model: None,
                lens_make: None,
                lens_model: None,
                iso: None,
                aperture: None,
                shutter_speed: None,
                focal_length: None,
                width: Some(1920),
                height: Some(1080),
                color_space: None,
                white_balance: None,
                exposure_mode: None,
                metering_mode: None,
                orientation: Some(1),
                flash_used: Some(false),
                latitude: None,
                longitude: None,
                location_name: None,

                thumbnail_path: None,
                has_thumbnail: Some(false),
                country: None,
                keywords: None,
                faces_detected: None,
                objects_detected: None,
                colors: None,
                duration: Some(0.3), // actual duration of downloaded video
                video_codec: Some("h264".to_string()),
                audio_codec: Some("aac".to_string()),
                bitrate: Some(1000),
                frame_rate: Some(30.0),
                is_favorite: Some(false),
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

            // Generate video thumbnail
            let result = generator
                .get_or_generate(&photo, ThumbnailSize::Medium)
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

            // Should be cached on disk
            let cache_key = CacheKey::from_photo(&photo, ThumbnailSize::Medium).unwrap();
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
            let metadata = crate::video_processor::extract_video_metadata(&video_path).await;

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
            // Test frame timing algorithm with different video durations
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

            // These will fail until we implement calculate_optimal_frame_time
            let short_time = crate::video_processor::calculate_optimal_frame_time(&short_video);
            let medium_time = crate::video_processor::calculate_optimal_frame_time(&medium_video);
            let long_time = crate::video_processor::calculate_optimal_frame_time(&long_video);

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

            // Generate different sizes
            let small = generator
                .get_or_generate(&photo, ThumbnailSize::Small)
                .await
                .unwrap();
            let medium = generator
                .get_or_generate(&photo, ThumbnailSize::Medium)
                .await
                .unwrap();
            let large = generator
                .get_or_generate(&photo, ThumbnailSize::Large)
                .await
                .unwrap();

            // All should succeed and be different sizes
            assert!(!small.is_empty());
            assert!(!medium.is_empty());
            assert!(!large.is_empty());

            // Larger thumbnails should generally have more data
            assert!(medium.len() >= small.len(), "Medium should be >= small");
            assert!(large.len() >= medium.len(), "Large should be >= medium");
        }
    }
}
