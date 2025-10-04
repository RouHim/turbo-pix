use image::{DynamicImage, ImageFormat};
use log::{debug, warn};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::fs;

use crate::config::Config;
use crate::db::{DbPool, Photo};
use crate::thumbnail_types::{CacheError, CacheKey, CacheResult, ThumbnailSize};
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
        let _cache_path = self.get_cache_path(&cache_key);
        self.save_to_disk_cache(&cache_key, &thumbnail_data).await?;

        // Update database to mark thumbnail as available
        // Note: Thumbnail status tracking removed as part of cleanup

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

    #[allow(dead_code)]
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
