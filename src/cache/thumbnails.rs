use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::{DynamicImage, ImageFormat};
use tokio::fs;
use tracing::{debug, warn};

use super::{CacheError, CacheKey, CacheResult, ThumbnailSize};
use crate::config::Config;
use crate::db::{DbPool, Photo};

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
        let cache_key = CacheKey::new(photo.id, size);

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

        let img = image::open(&photo_path)?;
        let thumbnail = self.resize_image(img, size);

        let thumbnail_data = self.encode_image(thumbnail)?;

        let cache_key = CacheKey::new(photo.id, size);
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
        let filename = format!("{}_{}.jpg", key.photo_id, key.size.as_str());

        let subdir = format!("{:03}", key.photo_id % 1000);
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
        let cache_key = CacheKey::new(1, ThumbnailSize::Small);
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
        let cache_dir = PathBuf::from(&config.cache.thumbnail_cache_path);
        let subdir = cache_dir.join("001"); // photo_id 1 % 1000 = 1, formatted as 001

        assert!(subdir.join("1_small.jpg").exists());
        assert!(subdir.join("1_medium.jpg").exists());
        assert!(subdir.join("1_large.jpg").exists());
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
        let cache_key = CacheKey::new(1, ThumbnailSize::Small);
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
}
