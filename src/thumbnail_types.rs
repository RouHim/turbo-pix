use std::fmt;
use std::str::FromStr;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailFormat {
    Jpeg,
    Webp,
}

impl ThumbnailFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThumbnailFormat::Jpeg => "jpeg",
            ThumbnailFormat::Webp => "webp",
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            ThumbnailFormat::Jpeg => "image/jpeg",
            ThumbnailFormat::Webp => "image/webp",
        }
    }
}

impl Default for ThumbnailFormat {
    fn default() -> Self {
        ThumbnailFormat::Jpeg
    }
}

impl FromStr for ThumbnailFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "jpeg" | "jpg" => Ok(ThumbnailFormat::Jpeg),
            "webp" => Ok(ThumbnailFormat::Webp),
            _ => Err(()),
        }
    }
}

impl fmt::Display for ThumbnailFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub content_hash: String,
    pub size: ThumbnailSize,
    pub format: ThumbnailFormat,
}

impl CacheKey {
    pub fn new(content_hash: String, size: ThumbnailSize, format: ThumbnailFormat) -> Self {
        Self {
            content_hash,
            size,
            format,
        }
    }

    pub fn from_photo(
        photo: &crate::db::Photo,
        size: ThumbnailSize,
        format: ThumbnailFormat,
    ) -> Result<Self, CacheError> {
        Ok(Self::new(photo.hash_sha256.clone(), size, format))
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}_{}", self.content_hash, self.size, self.format)
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
    #[error("Video processing error: {0}")]
    VideoProcessingError(String),
    #[error("Video metadata extraction failed: {0}")]
    VideoMetadataError(String),
}

pub type CacheResult<T> = Result<T, CacheError>;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub duration: f64,
    pub width: i32,
    pub height: i32,
}
