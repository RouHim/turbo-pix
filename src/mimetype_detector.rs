use std::path::Path;

/// Detects MIME type based on file extension
pub fn from_path(path: &Path) -> Option<MimeType> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(from_extension)
}

/// Detects MIME type from file extension string
fn from_extension(ext: &str) -> Option<MimeType> {
    let ext_lower = ext.to_lowercase();
    match ext_lower.as_str() {
        // Images
        "jpg" | "jpeg" => Some(MimeType::new("image", "jpeg")),
        "png" => Some(MimeType::new("image", "png")),
        "gif" => Some(MimeType::new("image", "gif")),
        "webp" => Some(MimeType::new("image", "webp")),
        "bmp" => Some(MimeType::new("image", "bmp")),
        "tiff" | "tif" => Some(MimeType::new("image", "tiff")),
        "heic" => Some(MimeType::new("image", "heic")),
        "raw" => Some(MimeType::new("image", "x-raw")),

        // Videos
        "mp4" => Some(MimeType::new("video", "mp4")),
        "mov" => Some(MimeType::new("video", "quicktime")),
        "avi" => Some(MimeType::new("video", "x-msvideo")),
        "mkv" => Some(MimeType::new("video", "x-matroska")),
        "webm" => Some(MimeType::new("video", "webm")),
        "m4v" => Some(MimeType::new("video", "x-m4v")),

        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MimeType {
    type_: String,
    subtype: String,
}

impl MimeType {
    fn new(type_: &str, subtype: &str) -> Self {
        Self {
            type_: type_.to_string(),
            subtype: subtype.to_string(),
        }
    }

    pub fn type_(&self) -> &str {
        &self.type_
    }

    pub fn subtype(&self) -> &str {
        &self.subtype
    }
}

impl std::fmt::Display for MimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.type_, self.subtype)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_image_types() {
        assert_eq!(
            from_path(&PathBuf::from("photo.jpg")).unwrap().to_string(),
            "image/jpeg"
        );
        assert_eq!(
            from_path(&PathBuf::from("photo.JPG")).unwrap().to_string(),
            "image/jpeg"
        );
        assert_eq!(
            from_path(&PathBuf::from("photo.png")).unwrap().to_string(),
            "image/png"
        );
        assert_eq!(
            from_path(&PathBuf::from("photo.webp")).unwrap().to_string(),
            "image/webp"
        );
    }

    #[test]
    fn test_video_types() {
        assert_eq!(
            from_path(&PathBuf::from("video.mp4")).unwrap().to_string(),
            "video/mp4"
        );
        assert_eq!(
            from_path(&PathBuf::from("video.mov")).unwrap().to_string(),
            "video/quicktime"
        );
        assert_eq!(
            from_path(&PathBuf::from("video.webm")).unwrap().to_string(),
            "video/webm"
        );
    }

    #[test]
    fn test_unknown_type() {
        assert!(from_path(&PathBuf::from("file.xyz")).is_none());
    }

    #[test]
    fn test_type_and_subtype() {
        let mime = from_path(&PathBuf::from("video.mp4")).unwrap();
        assert_eq!(mime.type_(), "video");
        assert_eq!(mime.subtype(), "mp4");
    }
}
