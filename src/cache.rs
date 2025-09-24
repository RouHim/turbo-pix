use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::{error, info};

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use tracing::debug;

use std::collections::HashMap;
use std::path::Path;

use image::{DynamicImage, ImageFormat};
use tokio::fs;
use tracing::warn;

use crate::config::Config;
use crate::db::{DbPool, Photo};

#[derive(Clone)]
pub struct CacheManager {
    memory_cache: MemoryCache,
    thumbnail_cache_dir: PathBuf,
}

impl CacheManager {
    pub fn new(memory_cache: MemoryCache, thumbnail_cache_dir: PathBuf) -> Self {
        Self {
            memory_cache,
            thumbnail_cache_dir,
        }
    }

    #[allow(dead_code)]
    pub async fn clear_for_path(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        info!("Clearing cache for deleted photo: {}", path);

        // Clear memory cache entries that might reference this path
        // Note: This is a simplified implementation - in practice you'd need
        // to track path-to-id mappings to clear specific entries

        // Clear thumbnail files that might exist for this path
        let filename = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        info!("Looking for thumbnails with filename: {}", filename);
        info!(
            "Thumbnail cache directory: {}",
            self.thumbnail_cache_dir.display()
        );

        for size in ["small", "medium", "large"] {
            let thumbnail_path = self
                .thumbnail_cache_dir
                .join(format!("{}_{}.jpg", filename, size));
            info!("Checking thumbnail path: {}", thumbnail_path.display());
            if thumbnail_path.exists() {
                if let Err(e) = std::fs::remove_file(&thumbnail_path) {
                    error!(
                        "Failed to remove thumbnail {}: {}",
                        thumbnail_path.display(),
                        e
                    );
                } else {
                    info!("Removed thumbnail: {}", thumbnail_path.display());
                }
            } else {
                info!("Thumbnail does not exist: {}", thumbnail_path.display());
            }
        }

        Ok(())
    }

    pub async fn clear_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Clearing all cache data");

        // Clear memory cache
        self.memory_cache.clear();

        // Clear thumbnail cache directory
        if self.thumbnail_cache_dir.exists() {
            let entries = std::fs::read_dir(&self.thumbnail_cache_dir)?;
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    if let Err(e) = std::fs::remove_file(entry.path()) {
                        error!(
                            "Failed to remove cache file {}: {}",
                            entry.path().display(),
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailSize {
    Small,  // 200px
    Medium, // 400px
    Large,  // 800px
}

impl ThumbnailSize {
    pub fn to_pixels(self) -> u32 {
        match self {
            ThumbnailSize::Small => 200,
            ThumbnailSize::Medium => 400,
            ThumbnailSize::Large => 800,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ThumbnailSize::Small => "small",
            ThumbnailSize::Medium => "medium",
            ThumbnailSize::Large => "large",
        }
    }
}

impl FromStr for ThumbnailSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "small" => Ok(ThumbnailSize::Small),
            "medium" => Ok(ThumbnailSize::Medium),
            "large" => Ok(ThumbnailSize::Large),
            _ => Err(()),
        }
    }
}

impl fmt::Display for ThumbnailSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub content_hash: String,
    pub size: ThumbnailSize,
}

impl CacheKey {
    pub fn new(content_hash: String, size: ThumbnailSize) -> Self {
        Self { content_hash, size }
    }

    pub fn from_photo(photo: &crate::db::Photo, size: ThumbnailSize) -> Result<Self, CacheError> {
        let hash = photo
            .hash_sha256
            .as_ref()
            .or(photo.hash_md5.as_ref())
            .ok_or(CacheError::MissingHash)?;

        Ok(Self::new(hash.clone(), size))
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.content_hash, self.size)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("Image processing error: {0}")]
    ImageError(#[from] image::ImageError),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Photo not found")]
    PhotoNotFound,
    #[error("Photo missing content hash - cannot cache without hash")]
    MissingHash,
    #[allow(dead_code)]
    #[error("Invalid thumbnail size")]
    InvalidSize,
    #[error("Video processing error: {0}")]
    VideoProcessingError(String),
    #[error("Video metadata extraction failed: {0}")]
    VideoMetadataError(String),
}

pub type CacheResult<T> = Result<T, CacheError>;

#[derive(Clone)]
pub struct MemoryCache {
    cache: Arc<Mutex<LruCache<String, Vec<u8>>>>,
    max_size_bytes: usize,
    current_size: Arc<Mutex<usize>>,
}

impl MemoryCache {
    pub fn new(capacity: usize, max_size_mb: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1000).unwrap());

        Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            max_size_bytes: max_size_mb * 1024 * 1024, // Convert MB to bytes
            current_size: Arc::new(Mutex::new(0)),
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let key_str = key.to_string();

        if let Ok(mut cache) = self.cache.lock() {
            if let Some(data) = cache.get(&key_str) {
                debug!("Memory cache hit for {}", key_str);
                return Some(data.clone());
            }
        }

        debug!("Memory cache miss for {}", key_str);
        None
    }

    pub fn put(&self, key: &CacheKey, data: Vec<u8>) -> CacheResult<()> {
        let key_str = key.to_string();
        let data_size = data.len();

        // Check if this single item would exceed our size limit
        if data_size > self.max_size_bytes {
            debug!("Item too large for memory cache: {} bytes", data_size);
            return Ok(());
        }

        // Acquire locks
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| std::io::Error::other("Failed to acquire cache lock"))?;

        let mut current_size = self
            .current_size
            .lock()
            .map_err(|_| std::io::Error::other("Failed to acquire size lock"))?;

        // Make room if needed
        while *current_size + data_size > self.max_size_bytes && !cache.is_empty() {
            if let Some((_, removed_data)) = cache.pop_lru() {
                *current_size = current_size.saturating_sub(removed_data.len());
                debug!(
                    "Evicted item from memory cache, new size: {} bytes",
                    *current_size
                );
            } else {
                break;
            }
        }

        // Add the new item
        if let Some(old_data) = cache.put(key_str.clone(), data) {
            // Replace existing item
            *current_size = current_size.saturating_sub(old_data.len()) + data_size;
        } else {
            // New item
            *current_size += data_size;
        }

        debug!(
            "Added {} to memory cache, total size: {} bytes",
            key_str, *current_size
        );
        Ok(())
    }

    #[allow(dead_code)]
    pub fn remove(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let key_str = key.to_string();

        if let (Ok(mut cache), Ok(mut current_size)) = (self.cache.lock(), self.current_size.lock())
        {
            if let Some(data) = cache.pop(&key_str) {
                *current_size = current_size.saturating_sub(data.len());
                debug!("Removed {} from memory cache", key_str);
                return Some(data);
            }
        }

        None
    }

    pub fn clear(&self) {
        if let (Ok(mut cache), Ok(mut current_size)) = (self.cache.lock(), self.current_size.lock())
        {
            cache.clear();
            *current_size = 0;
            debug!("Cleared memory cache");
        }
    }

    pub fn stats(&self) -> (usize, usize, usize) {
        if let (Ok(cache), Ok(current_size)) = (self.cache.lock(), self.current_size.lock()) {
            let len = cache.len();
            let cap = cache.cap().get();
            let size = *current_size;

            (len, cap, size)
        } else {
            (0, 0, 0)
        }
    }

    pub fn hit_rate(&self) -> f64 {
        // This is a simplified implementation
        // In a real system, you'd track hits/misses over time
        if let Ok(cache) = self.cache.lock() {
            if cache.cap().get() > 0 {
                cache.len() as f64 / cache.cap().get() as f64
            } else {
                0.0
            }
        } else {
            0.0
        }
    }
}

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

    #[test]
    fn test_thumbnail_size_conversions() {
        assert_eq!(ThumbnailSize::Small.to_pixels(), 200);
        assert_eq!(ThumbnailSize::Medium.to_pixels(), 400);
        assert_eq!(ThumbnailSize::Large.to_pixels(), 800);

        assert_eq!(ThumbnailSize::Small.as_str(), "small");
        assert_eq!(ThumbnailSize::Medium.as_str(), "medium");
        assert_eq!(ThumbnailSize::Large.as_str(), "large");

        assert_eq!("small".parse::<ThumbnailSize>(), Ok(ThumbnailSize::Small));
        assert_eq!("medium".parse::<ThumbnailSize>(), Ok(ThumbnailSize::Medium));
        assert_eq!("large".parse::<ThumbnailSize>(), Ok(ThumbnailSize::Large));
        assert_eq!("invalid".parse::<ThumbnailSize>(), Err(()));
    }

    #[test]
    fn test_thumbnail_size_display() {
        assert_eq!(format!("{}", ThumbnailSize::Small), "small");
        assert_eq!(format!("{}", ThumbnailSize::Medium), "medium");
        assert_eq!(format!("{}", ThumbnailSize::Large), "large");
    }

    #[test]
    fn test_cache_key() {
        let key = CacheKey::new("abcd1234".to_string(), ThumbnailSize::Medium);
        assert_eq!(key.content_hash, "abcd1234");
        assert_eq!(key.size, ThumbnailSize::Medium);
        assert_eq!(format!("{}", key), "abcd1234_medium");
    }

    #[test]
    fn test_cache_key_equality() {
        let key1 = CacheKey::new("test_hash_1".to_string(), ThumbnailSize::Small);
        let key2 = CacheKey::new("test_hash_1".to_string(), ThumbnailSize::Small);
        let key3 = CacheKey::new("test_hash_1".to_string(), ThumbnailSize::Medium);
        let key4 = CacheKey::new("test_hash_2".to_string(), ThumbnailSize::Small);

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
    }

    #[test]
    fn test_thumbnail_size_ordering() {
        let sizes = vec![
            ThumbnailSize::Large,
            ThumbnailSize::Small,
            ThumbnailSize::Medium,
        ];
        let mut sorted_sizes = sizes.clone();
        sorted_sizes.sort_by_key(|s| s.to_pixels());

        assert_eq!(
            sorted_sizes,
            vec![
                ThumbnailSize::Small,
                ThumbnailSize::Medium,
                ThumbnailSize::Large
            ]
        );
    }

    #[test]
    fn test_memory_cache_basic_operations() {
        let cache = MemoryCache::new(10, 1); // 10 items, 1MB
        let key = CacheKey::new("test_hash_basic".to_string(), ThumbnailSize::Small);
        let data = vec![1, 2, 3, 4, 5];

        // Initially empty
        assert!(cache.get(&key).is_none());

        // Put and get
        cache.put(&key, data.clone()).unwrap();
        assert_eq!(cache.get(&key), Some(data));

        // Stats
        let (len, cap, size) = cache.stats();
        assert_eq!(len, 1);
        assert_eq!(cap, 10);
        assert_eq!(size, 5);
    }

    #[test]
    fn test_memory_cache_eviction() {
        let cache = MemoryCache::new(2, 1); // 2 items max
        let key1 = CacheKey::new("test_hash_evict1".to_string(), ThumbnailSize::Small);
        let key2 = CacheKey::new("test_hash_evict2".to_string(), ThumbnailSize::Small);
        let key3 = CacheKey::new("test_hash_evict3".to_string(), ThumbnailSize::Small);
        let data = vec![0; 100]; // 100 bytes each

        // Fill cache
        cache.put(&key1, data.clone()).unwrap();
        cache.put(&key2, data.clone()).unwrap();

        // Both should be present
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_some());

        // Add third item, should evict first (LRU)
        cache.put(&key3, data.clone()).unwrap();

        // key1 should be evicted, key2 and key3 should be present
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_some());
        assert!(cache.get(&key3).is_some());
    }

    #[test]
    fn test_memory_cache_eviction_detailed() {
        let cache = MemoryCache::new(2, 1); // 2 items max
        let key1 = CacheKey::new("test_hash_1".to_string(), ThumbnailSize::Small);
        let key2 = CacheKey::new("test_hash_2".to_string(), ThumbnailSize::Small);
        let key3 = CacheKey::new("test_hash_3".to_string(), ThumbnailSize::Small);
        let data = vec![0; 100]; // 100 bytes each

        // Fill cache
        cache.put(&key1, data.clone()).unwrap();
        cache.put(&key2, data.clone()).unwrap();

        // Both should be present
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_some());

        // Add third item, should evict first (LRU)
        cache.put(&key3, data.clone()).unwrap();

        // key1 should be evicted, key2 and key3 should be present
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_some());
        assert!(cache.get(&key3).is_some());
    }

    #[test]
    fn test_memory_cache_size_limit() {
        let cache = MemoryCache::new(10, 1); // 1MB = 1,048,576 bytes
        let key = CacheKey::new("test_hash_large".to_string(), ThumbnailSize::Large);
        let large_data = vec![0; 2 * 1024 * 1024]; // 2MB data (too large)
        let small_data = vec![0; 1000]; // 1KB data

        // Large data should be rejected
        cache.put(&key, large_data).unwrap();
        assert!(cache.get(&key).is_none());

        // Small data should work
        cache.put(&key, small_data.clone()).unwrap();
        assert_eq!(cache.get(&key), Some(small_data));
    }

    #[test]
    fn test_memory_cache_remove() {
        let cache = MemoryCache::new(10, 1);
        let key = CacheKey::new("test_hash_remove".to_string(), ThumbnailSize::Medium);
        let data = vec![1, 2, 3];

        cache.put(&key, data.clone()).unwrap();
        assert_eq!(cache.get(&key), Some(data.clone()));

        let removed = cache.remove(&key);
        assert_eq!(removed, Some(data));
        assert!(cache.get(&key).is_none());

        let (len, _, size) = cache.stats();
        assert_eq!(len, 0);
        assert_eq!(size, 0);
    }

    #[test]
    fn test_memory_cache_clear() {
        let cache = MemoryCache::new(10, 1);
        let key1 = CacheKey::new("test_hash_clear1".to_string(), ThumbnailSize::Small);
        let key2 = CacheKey::new("test_hash_clear2".to_string(), ThumbnailSize::Medium);
        let data = vec![1, 2, 3];

        cache.put(&key1, data.clone()).unwrap();
        cache.put(&key2, data.clone()).unwrap();

        let (len, _, size) = cache.stats();
        assert_eq!(len, 2);
        assert!(size > 0);

        cache.clear();

        let (len, _, size) = cache.stats();
        assert_eq!(len, 0);
        assert_eq!(size, 0);
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_none());
    }

    #[test]
    fn test_memory_cache_replace_existing() {
        let cache = MemoryCache::new(10, 1);
        let key = CacheKey::new("test_hash_replace".to_string(), ThumbnailSize::Small);
        let data1 = vec![1, 2, 3];
        let data2 = vec![4, 5, 6, 7];

        cache.put(&key, data1).unwrap();
        cache.put(&key, data2.clone()).unwrap();

        assert_eq!(cache.get(&key), Some(data2));

        let (len, _, size) = cache.stats();
        assert_eq!(len, 1);
        assert_eq!(size, 4); // Should be size of second data
    }

    mod thumbnail_tests {
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
                    thumbnail_cache_path: cache_path
                        .join("thumbnails")
                        .to_string_lossy()
                        .to_string(),
                    memory_cache_size: 100,
                    memory_cache_max_size_mb: 10,
                },
                thumbnail_sizes: vec![200, 400, 800],
                workers: 1,
                max_connections: 10,
                cache_size_mb: 100,
                scan_interval: 3600,
                batch_size: 1000,
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
}
