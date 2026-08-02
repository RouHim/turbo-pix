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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThumbnailFormat {
    #[default]
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
    /// Content version (file size + mtime) folded into the cache filename:
    /// the DB hash is derived from the FILE PATH (favorites survive
    /// re-exports), so a file edited in place keeps its hash while its bytes
    /// change — without a version, the old thumbnail would be served forever
    /// from the disk cache. Empty for unversioned keys (tests, legacy).
    pub content_version: String,
    pub size: ThumbnailSize,
    pub format: ThumbnailFormat,
}

impl CacheKey {
    pub fn new(content_hash: String, size: ThumbnailSize, format: ThumbnailFormat) -> Self {
        Self {
            content_hash,
            content_version: String::new(),
            size,
            format,
        }
    }

    pub fn from_photo(
        photo: &crate::db::Photo,
        size: ThumbnailSize,
        format: ThumbnailFormat,
    ) -> Result<Self, CacheError> {
        let content_version = format!(
            "{}_{}",
            photo.file_size,
            photo.date_modified.timestamp_millis()
        );
        Ok(Self {
            content_hash: photo.hash_sha256.clone(),
            content_version,
            size,
            format,
        })
    }

    pub fn filename(&self) -> String {
        if self.content_version.is_empty() {
            format!(
                "{}_{}.{}",
                self.content_hash,
                self.size.as_str(),
                self.format.as_str()
            )
        } else {
            format!(
                "{}_{}_{}.{}",
                self.content_hash,
                self.content_version,
                self.size.as_str(),
                self.format.as_str()
            )
        }
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.content_version.is_empty() {
            write!(f, "{}_{}_{}", self.content_hash, self.size, self.format)
        } else {
            write!(
                f,
                "{}_{}_{}_{}",
                self.content_hash, self.content_version, self.size, self.format
            )
        }
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
