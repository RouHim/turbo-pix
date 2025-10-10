use image::{DynamicImage, ImageFormat};
use log::{debug, warn};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::fs;

use crate::config::Config;
use crate::db::{DbPool, Photo};
use crate::thumbnail_types::{CacheError, CacheKey, CacheResult, ThumbnailFormat, ThumbnailSize};
use crate::video_processor;

#[derive(Clone, Debug)]
struct CacheEntry {
    path: PathBuf,
    last_access: SystemTime,
    file_size: u64,
}

#[derive(Clone)]
pub struct ThumbnailGenerator {
    cache_dir: PathBuf,
    disk_cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    max_cache_size_bytes: u64,
}

impl ThumbnailGenerator {
    pub fn new(config: &Config, _db_pool: DbPool) -> CacheResult<Self> {
        let cache_dir = PathBuf::from(&config.cache.thumbnail_cache_path);

        std::fs::create_dir_all(&cache_dir)?;

        // Convert MB to bytes
        let max_cache_size_bytes = config.cache.max_cache_size_mb * 1024 * 1024;

        Ok(Self {
            cache_dir,
            disk_cache: Arc::new(Mutex::new(HashMap::new())),
            max_cache_size_bytes,
        })
    }

    pub async fn get_or_generate(
        &self,
        photo: &Photo,
        size: ThumbnailSize,
        format: ThumbnailFormat,
    ) -> CacheResult<Vec<u8>> {
        let cache_key = CacheKey::from_photo(photo, size, format)?;

        if let Some(cached_data) = self.get_from_disk_cache(&cache_key).await {
            debug!("Cache hit for {}", cache_key);
            return Ok(cached_data);
        }

        debug!("Cache miss for {}, generating thumbnail", cache_key);
        self.generate_thumbnail(photo, size, format).await
    }

    async fn get_from_disk_cache(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let cache_path = self.get_cache_path(key);

        match fs::read(&cache_path).await {
            Ok(data) => {
                let file_size = data.len() as u64;
                let now = SystemTime::now();

                // Update access time in cache index
                if let Ok(mut cache) = self.disk_cache.lock() {
                    cache.insert(
                        key.to_string(),
                        CacheEntry {
                            path: cache_path,
                            last_access: now,
                            file_size,
                        },
                    );
                }

                Some(data)
            }
            Err(_) => None,
        }
    }

    async fn generate_thumbnail(
        &self,
        photo: &Photo,
        size: ThumbnailSize,
        format: ThumbnailFormat,
    ) -> CacheResult<Vec<u8>> {
        let photo_path = PathBuf::from(&photo.file_path);

        if !photo_path.exists() {
            return Err(CacheError::PhotoNotFound);
        }

        let thumbnail_data = if self.is_video_file(photo) {
            self.generate_video_thumbnail(&photo_path, size, photo.orientation, format)
                .await?
        } else {
            let img = image::open(&photo_path)?;
            let img = self.apply_orientation(img, photo.orientation);
            let thumbnail = self.resize_image(img, size);
            self.encode_image(thumbnail, format)?
        };

        let cache_key = CacheKey::from_photo(photo, size, format)?;
        let _cache_path = self.get_cache_path(&cache_key);
        self.save_to_disk_cache(&cache_key, &thumbnail_data).await?;

        // Update database to mark thumbnail as available
        // Note: Thumbnail status tracking removed as part of cleanup

        Ok(thumbnail_data)
    }

    fn apply_orientation(&self, img: DynamicImage, orientation: Option<i32>) -> DynamicImage {
        match orientation {
            Some(2) => img.fliph(),
            Some(3) => img.rotate180(),
            Some(4) => img.flipv(),
            Some(5) => img.fliph().rotate90(),
            Some(6) => img.rotate90(),
            Some(7) => img.fliph().rotate270(),
            Some(8) => img.rotate270(),
            _ => img, // 1 or None = no transformation needed
        }
    }

    fn resize_image(&self, img: DynamicImage, size: ThumbnailSize) -> DynamicImage {
        let target_size = size.to_pixels();

        img.thumbnail(target_size, target_size)
    }

    fn encode_image(&self, img: DynamicImage, format: ThumbnailFormat) -> CacheResult<Vec<u8>> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let image_format = match format {
            ThumbnailFormat::Jpeg => ImageFormat::Jpeg,
            ThumbnailFormat::Webp => ImageFormat::WebP,
        };
        img.write_to(&mut buffer, image_format)?;
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
        orientation: Option<i32>,
        format: ThumbnailFormat,
    ) -> CacheResult<Vec<u8>> {
        // Extract video metadata to get duration
        let metadata = video_processor::extract_video_metadata(video_path).await?;

        // Calculate optimal frame extraction time
        let frame_time = video_processor::calculate_optimal_frame_time(&metadata);

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
        video_processor::extract_frame_at_time(video_path, frame_time, &temp_frame_path).await?;

        // Load the extracted frame and create thumbnail
        let img = image::open(&temp_frame_path).map_err(|e| {
            CacheError::VideoProcessingError(format!("Failed to load extracted frame: {}", e))
        })?;

        let img = self.apply_orientation(img, orientation);
        let thumbnail = self.resize_image(img, size);
        let thumbnail_data = self.encode_image(thumbnail, format)?;

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

        let file_size = data.len() as u64;
        let now = SystemTime::now();

        // Update cache index with metadata
        if let Ok(mut cache) = self.disk_cache.lock() {
            cache.insert(
                key.to_string(),
                CacheEntry {
                    path: cache_path.clone(),
                    last_access: now,
                    file_size,
                },
            );
        }

        debug!("Saved thumbnail to cache: {:?}", cache_path);

        // Enforce cache limit after saving
        self.enforce_cache_limit().await?;

        Ok(())
    }

    pub fn get_cache_path(&self, key: &CacheKey) -> PathBuf {
        let filename = format!("{}_{}.jpg", key.content_hash, key.size.as_str());

        // Use first 3 characters of hash for subdirectory distribution
        let subdir = if key.content_hash.len() >= 3 {
            key.content_hash[..3].to_string()
        } else {
            key.content_hash.clone()
        };

        self.cache_dir.join(subdir).join(filename)
    }

    fn get_current_cache_size(&self) -> u64 {
        if let Ok(cache) = self.disk_cache.lock() {
            cache.values().map(|entry| entry.file_size).sum()
        } else {
            0
        }
    }

    async fn enforce_cache_limit(&self) -> CacheResult<()> {
        let current_size = self.get_current_cache_size();

        if current_size <= self.max_cache_size_bytes {
            return Ok(());
        }

        debug!(
            "Cache size {}MB exceeds limit {}MB, evicting oldest entries",
            current_size / 1024 / 1024,
            self.max_cache_size_bytes / 1024 / 1024
        );

        // Get all entries sorted by last access time (oldest first)
        let mut entries_to_evict = Vec::new();

        if let Ok(cache) = self.disk_cache.lock() {
            let mut sorted_entries: Vec<_> = cache.iter().collect();
            sorted_entries.sort_by_key(|(_, entry)| entry.last_access);

            let mut size_to_free = current_size - self.max_cache_size_bytes;
            for (key, entry) in sorted_entries {
                if size_to_free == 0 {
                    break;
                }
                entries_to_evict.push((key.clone(), entry.clone()));
                size_to_free = size_to_free.saturating_sub(entry.file_size);
            }
        }

        // Delete files and update cache index
        for (key, entry) in entries_to_evict {
            if let Err(e) = fs::remove_file(&entry.path).await {
                warn!("Failed to remove cache file {:?}: {}", entry.path, e);
            } else {
                debug!("Evicted cache entry: {:?}", entry.path);
            }

            if let Ok(mut cache) = self.disk_cache.lock() {
                cache.remove(&key);
            }
        }

        Ok(())
    }

    // Test helper methods
    #[cfg(test)]
    pub async fn clear_cache(&self) -> CacheResult<()> {
        fs::remove_dir_all(&self.cache_dir).await?;
        fs::create_dir_all(&self.cache_dir).await?;

        if let Ok(mut cache) = self.disk_cache.lock() {
            cache.clear();
        }

        Ok(())
    }

    #[cfg(test)]
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

    #[cfg(test)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CacheConfig, Config};
    use crate::db::{create_in_memory_pool, Photo};
    use chrono::Utc;
    use tempfile::TempDir;

    const TEST_PORT: u16 = 8080;

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
        };

        (config, temp_dir)
    }

    fn create_test_image(path: &std::path::Path) -> std::io::Result<()> {
        use image::{ImageBuffer, Rgb};

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(10, 10, |_x, _y| Rgb([255, 0, 0]));

        img.save(path).map_err(std::io::Error::other)?;
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
            blurhash: None,
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

        let image_path = temp_dir.path().join("test.jpg");
        create_test_image(&image_path).unwrap();

        let photo = create_test_photo(&image_path.to_string_lossy());

        let result = generator
            .get_or_generate(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg)
            .await;
        assert!(result.is_ok());

        let thumbnail_data = result.unwrap();
        assert!(!thumbnail_data.is_empty());

        let cache_key =
            CacheKey::from_photo(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg).unwrap();
        let cache_path = generator.get_cache_path(&cache_key);
        assert!(cache_path.exists());
    }

    #[tokio::test]
    async fn test_thumbnail_cache_hit() {
        let (config, temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        let image_path = temp_dir.path().join("test.jpg");
        create_test_image(&image_path).unwrap();

        let photo = create_test_photo(&image_path.to_string_lossy());

        let result1 = generator
            .get_or_generate(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        let result2 = generator
            .get_or_generate(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        assert_eq!(result1, result2);
    }

    #[tokio::test]
    async fn test_thumbnail_different_sizes() {
        let (config, temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        let image_path = temp_dir.path().join("test.jpg");
        create_test_image(&image_path).unwrap();

        let photo = create_test_photo(&image_path.to_string_lossy());

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

        let small_key =
            CacheKey::from_photo(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg).unwrap();
        let medium_key =
            CacheKey::from_photo(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg).unwrap();
        let large_key =
            CacheKey::from_photo(&photo, ThumbnailSize::Large, ThumbnailFormat::Jpeg).unwrap();

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
            .get_or_generate(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg)
            .await;
        assert!(matches!(result, Err(CacheError::PhotoNotFound)));
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let (config, temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        let image_path = temp_dir.path().join("test.jpg");
        create_test_image(&image_path).unwrap();

        let photo = create_test_photo(&image_path.to_string_lossy());
        generator
            .get_or_generate(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        let cache_key =
            CacheKey::from_photo(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg).unwrap();
        let cache_path = generator.get_cache_path(&cache_key);
        assert!(cache_path.exists());

        generator.clear_cache().await.unwrap();

        assert!(!cache_path.exists());

        assert!(std::path::PathBuf::from(&config.cache.thumbnail_cache_path).exists());
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let (config, temp_dir) = create_test_config();
        let db_pool = create_in_memory_pool().unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        let (files, size) = generator.get_cache_stats().await;
        assert_eq!(files, 0);
        assert_eq!(size, 0);

        let image_path = temp_dir.path().join("test.jpg");
        create_test_image(&image_path).unwrap();

        let photo = create_test_photo(&image_path.to_string_lossy());
        generator
            .get_or_generate(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg)
            .await
            .unwrap();
        generator
            .get_or_generate(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        let (files, size) = generator.get_cache_stats().await;
        assert_eq!(files, 2);
        assert!(size > 0);
    }

    #[tokio::test]
    async fn test_cache_limit_enforcement() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("cache");

        let config = Config {
            port: TEST_PORT,
            photo_paths: vec![],
            data_path: temp_dir.path().to_string_lossy().to_string(),
            db_path: temp_dir
                .path()
                .join("database/turbo-pix.db")
                .to_string_lossy()
                .to_string(),
            cache: CacheConfig {
                thumbnail_cache_path: cache_path.join("thumbnails").to_string_lossy().to_string(),
                max_cache_size_mb: 1,
            },
        };

        let db_pool = create_in_memory_pool().unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        for i in 0..20 {
            let image_path = temp_dir.path().join(format!("test_{}.jpg", i));
            create_test_image(&image_path).unwrap();

            let mut photo = create_test_photo(&image_path.to_string_lossy());
            photo.hash_sha256 = format!("{:0>64}", i);

            let _ = generator
                .get_or_generate(&photo, ThumbnailSize::Small, ThumbnailFormat::Jpeg)
                .await;
            let _ = generator
                .get_or_generate(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg)
                .await;
            let _ = generator
                .get_or_generate(&photo, ThumbnailSize::Large, ThumbnailFormat::Jpeg)
                .await;
        }

        let (files, total_size) = generator.get_cache_stats().await;
        let max_bytes = 1024 * 1024;

        assert!(
            total_size <= max_bytes,
            "Cache size {}MB should be <= 1MB limit (found {} files)",
            total_size / 1024 / 1024,
            files
        );
    }

    #[tokio::test]
    async fn test_lru_eviction_order() {
        use tokio::time::{sleep, Duration};

        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("cache");

        let config = Config {
            port: TEST_PORT,
            photo_paths: vec![],
            data_path: temp_dir.path().to_string_lossy().to_string(),
            db_path: temp_dir
                .path()
                .join("database/turbo-pix.db")
                .to_string_lossy()
                .to_string(),
            cache: CacheConfig {
                thumbnail_cache_path: cache_path.join("thumbnails").to_string_lossy().to_string(),
                max_cache_size_mb: 1,
            },
        };

        let db_pool = create_in_memory_pool().unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        let image1 = temp_dir.path().join("test_1.jpg");
        let image2 = temp_dir.path().join("test_2.jpg");
        let image3 = temp_dir.path().join("test_3.jpg");

        create_test_image(&image1).unwrap();
        create_test_image(&image2).unwrap();
        create_test_image(&image3).unwrap();

        let mut photo1 = create_test_photo(&image1.to_string_lossy());
        let mut photo2 = create_test_photo(&image2.to_string_lossy());
        let mut photo3 = create_test_photo(&image3.to_string_lossy());

        photo1.hash_sha256 = "1".repeat(64);
        photo2.hash_sha256 = "2".repeat(64);
        photo3.hash_sha256 = "3".repeat(64);

        generator
            .get_or_generate(&photo1, ThumbnailSize::Large, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        generator
            .get_or_generate(&photo2, ThumbnailSize::Large, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        generator
            .get_or_generate(&photo1, ThumbnailSize::Large, ThumbnailFormat::Jpeg)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        for i in 4..25 {
            let image_path = temp_dir.path().join(format!("test_{}.jpg", i));
            create_test_image(&image_path).unwrap();

            let mut photo = create_test_photo(&image_path.to_string_lossy());
            photo.hash_sha256 = format!("{:0>64}", i);

            generator
                .get_or_generate(&photo, ThumbnailSize::Large, ThumbnailFormat::Jpeg)
                .await
                .unwrap();
        }

        let cache_key1 =
            CacheKey::from_photo(&photo1, ThumbnailSize::Large, ThumbnailFormat::Jpeg).unwrap();
        let cache_key2 =
            CacheKey::from_photo(&photo2, ThumbnailSize::Large, ThumbnailFormat::Jpeg).unwrap();

        let path1_exists = generator.get_cache_path(&cache_key1).exists();
        let path2_exists = generator.get_cache_path(&cache_key2).exists();

        // Both exist or both don't exist: cache limit not exceeded
        // If only one exists, it should be path1 (more recently accessed)
        if path1_exists != path2_exists {
            assert!(
                path1_exists && !path2_exists,
                "LRU eviction should keep more recently accessed items"
            );
        }
    }

    #[tokio::test]
    async fn test_custom_cache_limit() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("cache");

        let config = Config {
            port: TEST_PORT,
            photo_paths: vec![],
            data_path: temp_dir.path().to_string_lossy().to_string(),
            db_path: temp_dir
                .path()
                .join("database/turbo-pix.db")
                .to_string_lossy()
                .to_string(),
            cache: CacheConfig {
                thumbnail_cache_path: cache_path.join("thumbnails").to_string_lossy().to_string(),
                max_cache_size_mb: 2,
            },
        };

        let db_pool = create_in_memory_pool().unwrap();
        let generator = ThumbnailGenerator::new(&config, db_pool).unwrap();

        for i in 0..30 {
            let image_path = temp_dir.path().join(format!("test_{}.jpg", i));
            create_test_image(&image_path).unwrap();

            let mut photo = create_test_photo(&image_path.to_string_lossy());
            photo.hash_sha256 = format!("{:0>64}", i);

            let _ = generator
                .get_or_generate(&photo, ThumbnailSize::Medium, ThumbnailFormat::Jpeg)
                .await;
        }

        let (_files, total_size) = generator.get_cache_stats().await;
        let max_bytes = 2 * 1024 * 1024;

        assert!(
            total_size <= max_bytes,
            "Cache size {}MB should be <= 2MB limit",
            total_size / 1024 / 1024
        );
    }
}
