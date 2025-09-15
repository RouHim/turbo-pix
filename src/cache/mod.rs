use std::fmt;
use std::str::FromStr;

pub mod memory;
pub mod thumbnails;

pub use memory::MemoryCache;
pub use thumbnails::ThumbnailGenerator;

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
    pub photo_id: i64,
    pub size: ThumbnailSize,
}

impl CacheKey {
    pub fn new(photo_id: i64, size: ThumbnailSize) -> Self {
        Self { photo_id, size }
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.photo_id, self.size)
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
    #[allow(dead_code)]
    #[error("Invalid thumbnail size")]
    InvalidSize,
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
        let key = CacheKey::new(123, ThumbnailSize::Medium);
        assert_eq!(key.photo_id, 123);
        assert_eq!(key.size, ThumbnailSize::Medium);
        assert_eq!(format!("{}", key), "123_medium");
    }

    #[test]
    fn test_cache_key_equality() {
        let key1 = CacheKey::new(1, ThumbnailSize::Small);
        let key2 = CacheKey::new(1, ThumbnailSize::Small);
        let key3 = CacheKey::new(1, ThumbnailSize::Medium);
        let key4 = CacheKey::new(2, ThumbnailSize::Small);

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
}
