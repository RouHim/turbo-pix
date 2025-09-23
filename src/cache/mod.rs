use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::{error, info};

pub mod memory;
pub mod thumbnails;

pub use memory::MemoryCache;
pub use thumbnails::ThumbnailGenerator;

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
        let hash = photo.hash_sha256.as_ref()
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
}
