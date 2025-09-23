use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::{DynamicImage, ImageFormat};
use tokio::fs;
use tracing::{debug, warn};

use super::{CacheError, CacheKey, CacheResult, ThumbnailSize};
use crate::config::Config;
use crate::db::{DbPool, Photo};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub duration: f64,
    pub width: i32,
    pub height: i32,
}

pub struct ThumbnailGenerator {
    cache_dir: PathBuf,
    disk_cache: Arc<Mutex<HashMap<String, PathBuf>>>,
    db_pool: DbPool,
}

impl ThumbnailGenerator {
    pub fn new(config: &Config, db_pool: DbPool) -> CacheResult<Self> {
        let cache_dir = PathBuf::from(&config.cache.thumbnail_cache_path);

        std::fs::create_dir_all(&cache_dir)?;

        Ok(Self {
            cache_dir,
            disk_cache: Arc::new(Mutex::new(HashMap::new())),
            db_pool,
        })
    }

    pub async fn get_or_generate(
        &self,
        photo: &Photo,
        size: ThumbnailSize,
    ) -> CacheResult<Vec<u8>> {
        let cache_key = CacheKey::from_photo(photo, size)?;

        if let Some(cached_data) = self.get_from_disk_cache(&cache_key).await {
            debug!("Cache hit for {}", cache_key);
            return Ok(cached_data);
        }

        debug!("Cache miss for {}, generating thumbnail", cache_key);
        self.generate_thumbnail(photo, size).await
    }

    async fn get_from_disk_cache(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let cache_path = self.get_cache_path(key);

        match fs::read(&cache_path).await {
            Ok(data) => {
                self.update_disk_cache_index(key.to_string(), cache_path);
                Some(data)
            }
            Err(_) => None,
        }
    }

    async fn generate_thumbnail(&self, photo: &Photo, size: ThumbnailSize) -> CacheResult<Vec<u8>> {
        let photo_path = PathBuf::from(&photo.file_path);

        if !photo_path.exists() {
            return Err(CacheError::PhotoNotFound);
        }

        let thumbnail_data = if self.is_video_file(photo) {
            self.generate_video_thumbnail(&photo_path, size).await?
        } else {
            let img = image::open(&photo_path)?;
            let thumbnail = self.resize_image(img, size);
            self.encode_image(thumbnail)?
        };

        let cache_key = CacheKey::from_photo(photo, size)?;
        let cache_path = self.get_cache_path(&cache_key);
        self.save_to_disk_cache(&cache_key, &thumbnail_data).await?;

        // Update database to mark thumbnail as available
        if let Err(e) = photo.update_thumbnail_status(
            &self.db_pool,
            true,
            Some(cache_path.to_string_lossy().to_string()),
        ) {
            warn!("Failed to update thumbnail status in database: {}", e);
        }

        Ok(thumbnail_data)
    }

    fn resize_image(&self, img: DynamicImage, size: ThumbnailSize) -> DynamicImage {
        let target_size = size.to_pixels();

        img.thumbnail(target_size, target_size)
    }

    fn encode_image(&self, img: DynamicImage) -> CacheResult<Vec<u8>> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buffer, ImageFormat::Jpeg)?;
        Ok(buffer.into_inner())
    }

    fn is_video_file(&self, photo: &Photo) -> bool {
        photo
            .mime_type
            .as_ref()
            .map(|mime| mime.starts_with("video/"))
            .unwrap_or(false)
    }

    async fn generate_video_thumbnail(
        &self,
        video_path: &Path,
        size: ThumbnailSize,
    ) -> CacheResult<Vec<u8>> {
        // Extract video metadata to get duration
        let metadata = Self::extract_video_metadata(video_path).await?;

        // Calculate optimal frame extraction time
        let frame_time = Self::calculate_optimal_frame_time(&metadata);

        // Create temporary file for extracted frame
        let temp_dir = std::env::temp_dir();
        let temp_frame_path = temp_dir.join(format!(
            "turbo_pix_frame_{}_{}.jpg",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));

        // Extract frame at calculated time
        Self::extract_frame_at_time(video_path, frame_time, &temp_frame_path).await?;

        // Load the extracted frame and create thumbnail
        let img = image::open(&temp_frame_path).map_err(|e| {
            CacheError::VideoProcessingError(format!("Failed to load extracted frame: {}", e))
        })?;

        let thumbnail = self.resize_image(img, size);
        let thumbnail_data = self.encode_image(thumbnail)?;

        // Clean up temporary file
        if temp_frame_path.exists() {
            let _ = std::fs::remove_file(&temp_frame_path);
        }

        Ok(thumbnail_data)
    }

    async fn save_to_disk_cache(&self, key: &CacheKey, data: &[u8]) -> CacheResult<()> {
        let cache_path = self.get_cache_path(key);

        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(&cache_path, data).await?;
        self.update_disk_cache_index(key.to_string(), cache_path.clone());

        debug!("Saved thumbnail to cache: {:?}", cache_path);
        Ok(())
    }

    fn get_cache_path(&self, key: &CacheKey) -> PathBuf {
        let filename = format!("{}_{}.jpg", key.content_hash, key.size.as_str());

        // Use first 3 characters of hash for subdirectory distribution
        let subdir = if key.content_hash.len() >= 3 {
            key.content_hash[..3].to_string()
        } else {
            key.content_hash.clone()
        };
        
        self.cache_dir.join(subdir).join(filename)
    }

    fn update_disk_cache_index(&self, key: String, path: PathBuf) {
        if let Ok(mut cache) = self.disk_cache.lock() {
            cache.insert(key, path);
        } else {
            warn!("Failed to acquire disk cache lock");
        }
    }

    pub async fn clear_cache(&self) -> CacheResult<()> {
        fs::remove_dir_all(&self.cache_dir).await?;
        fs::create_dir_all(&self.cache_dir).await?;

        if let Ok(mut cache) = self.disk_cache.lock() {
            cache.clear();
        }

        debug!("Cleared thumbnail cache");
        Ok(())
    }

    pub async fn get_cache_stats(&self) -> (usize, u64) {
        let mut total_files = 0;
        let mut total_size = 0;

        if let Ok(mut entries) = fs::read_dir(&self.cache_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(metadata) = entry.metadata().await {
                    if metadata.is_file() {
                        total_files += 1;
                        total_size += metadata.len();
                    } else if metadata.is_dir() {
                        // Recursively check subdirectories
                        let (subdir_files, subdir_size) =
                            self.get_subdir_stats(&entry.path()).await;
                        total_files += subdir_files;
                        total_size += subdir_size;
                    }
                }
            }
        }

        (total_files, total_size)
    }

    async fn get_subdir_stats(&self, dir_path: &Path) -> (usize, u64) {
        let mut files = 0;
        let mut size = 0;

        if let Ok(mut entries) = fs::read_dir(dir_path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(metadata) = entry.metadata().await {
                    if metadata.is_file() {
                        files += 1;
                        size += metadata.len();
                    }
                }
            }
        }

        (files, size)
    }

    // Video processing methods
    pub async fn extract_video_metadata(video_path: &Path) -> CacheResult<VideoMetadata> {
        use std::process::Command;

        let output = Command::new("ffprobe")
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
            .map_err(|e| CacheError::VideoProcessingError(format!("ffprobe failed: {}", e)))?;

        if !output.status.success() {
            return Err(CacheError::VideoProcessingError(format!(
                "ffprobe exited with status: {}",
                output.status
            )));
        }

        let json_str = String::from_utf8(output.stdout).map_err(|e| {
            CacheError::VideoProcessingError(format!("Invalid UTF-8 output: {}", e))
        })?;

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
        use std::process::Command;

        let output = Command::new("ffmpeg")
            .args([
                "-y", // Overwrite output file
                "-i",
                video_path.to_str().unwrap(),
                "-ss",
                &time_seconds.to_string(),
                "-frames:v",
                "1",
                "-q:v",
                "2", // High quality
                output_path.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| CacheError::VideoProcessingError(format!("ffmpeg failed: {}", e)))?;

        if !output.status.success() {
            return Err(CacheError::VideoProcessingError(format!(
                "ffmpeg exited with status: {}",
                output.status
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CacheConfig, Config};
    use crate::db::create_in_memory_pool;
    use chrono::Utc;
    use tempfile::TempDir;

    fn create_test_config() -> (Config, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("cache");

        let config = Config {
            port: 8080,
            host: "localhost".to_string(),
            photo_paths: vec![],
            db_path: "test.db".to_string(),
            cache_path: cache_path.to_string_lossy().to_string(),
            cache: CacheConfig {
                thumbnail_cache_path: cache_path.join("thumbnails").to_string_lossy().to_string(),
                memory_cache_size: 100,
                memory_cache_max_size_mb: 10,
            },
            thumbnail_sizes: vec![200, 400, 800],
            workers: 1,
            max_connections: 10,
            cache_size_mb: 100,
            scan_interval: 3600,
            batch_size: 1000,
            metrics_enabled: false,
            health_check_path: "/health".to_string(),
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
            id: 1,
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
            hash_md5: Some("test_hash_md5".to_string()),
            hash_sha256: Some("test_hash_sha256".to_string()),
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
            .parse::<PathBuf>()
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
        assert!(PathBuf::from(&config.cache.thumbnail_cache_path).exists());
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
            id: 2,
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
            width: Some(320),
            height: Some(240),
            color_space: None,
            white_balance: None,
            exposure_mode: None,
            metering_mode: None,
            orientation: Some(1),
            flash_used: Some(false),
            latitude: None,
            longitude: None,
            location_name: None,
            hash_md5: Some("test_video_hash_md5".to_string()),
            hash_sha256: Some("test_video_hash_sha256".to_string()),
            thumbnail_path: None,
            has_thumbnail: Some(false),
            country: None,
            keywords: None,
            faces_detected: None,
            objects_detected: None,
            colors: None,
            duration: Some(5.0), // 5 second test video
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

        // Use actual test video file from photos directory
        let video_path = "/home/rouven/projects/turbo-pix/photos/test_video.mp4";
        let photo = create_test_video_photo(video_path);

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
        let video_path = "/home/rouven/projects/turbo-pix/photos/test_video.mp4";

        // This test will fail until we implement extract_video_metadata
        let metadata =
            ThumbnailGenerator::extract_video_metadata(std::path::Path::new(video_path)).await;

        assert!(
            metadata.is_ok(),
            "Should extract video metadata successfully"
        );
        let metadata = metadata.unwrap();

        assert!(metadata.duration > 0.0, "Duration should be positive");
        assert_eq!(metadata.width, 320, "Width should match expected");
        assert_eq!(metadata.height, 240, "Height should match expected");
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
        let short_time = ThumbnailGenerator::calculate_optimal_frame_time(&short_video);
        let medium_time = ThumbnailGenerator::calculate_optimal_frame_time(&medium_video);
        let long_time = ThumbnailGenerator::calculate_optimal_frame_time(&long_video);

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

        let video_path = "/home/rouven/projects/turbo-pix/photos/test_video.mp4";
        let photo = create_test_video_photo(video_path);

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
