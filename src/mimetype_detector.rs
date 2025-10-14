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

        // RAW formats - Canon
        "cr2" => Some(MimeType::new("image", "x-canon-cr2")),
        "cr3" => Some(MimeType::new("image", "x-canon-cr3")),

        // RAW formats - Nikon
        "nef" => Some(MimeType::new("image", "x-nikon-nef")),
        "nrw" => Some(MimeType::new("image", "x-nikon-nrw")),

        // RAW formats - Sony
        "arw" => Some(MimeType::new("image", "x-sony-arw")),
        "srf" => Some(MimeType::new("image", "x-sony-srf")),
        "sr2" => Some(MimeType::new("image", "x-sony-sr2")),

        // RAW formats - Fujifilm
        "raf" => Some(MimeType::new("image", "x-fuji-raf")),

        // RAW formats - Olympus
        "orf" => Some(MimeType::new("image", "x-olympus-orf")),

        // RAW formats - Panasonic
        "rw2" => Some(MimeType::new("image", "x-panasonic-rw2")),

        // RAW formats - Adobe & Others
        "dng" => Some(MimeType::new("image", "x-adobe-dng")),
        "pef" => Some(MimeType::new("image", "x-pentax-pef")),

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
    fn test_raw_types() {
        // Canon
        assert_eq!(
            from_path(&PathBuf::from("photo.cr2")).unwrap().to_string(),
            "image/x-canon-cr2"
        );
        assert_eq!(
            from_path(&PathBuf::from("photo.CR2")).unwrap().to_string(),
            "image/x-canon-cr2"
        );
        assert_eq!(
            from_path(&PathBuf::from("photo.cr3")).unwrap().to_string(),
            "image/x-canon-cr3"
        );

        // Nikon
        assert_eq!(
            from_path(&PathBuf::from("photo.nef")).unwrap().to_string(),
            "image/x-nikon-nef"
        );
        assert_eq!(
            from_path(&PathBuf::from("photo.NEF")).unwrap().to_string(),
            "image/x-nikon-nef"
        );

        // Sony
        assert_eq!(
            from_path(&PathBuf::from("photo.arw")).unwrap().to_string(),
            "image/x-sony-arw"
        );

        // Adobe DNG
        assert_eq!(
            from_path(&PathBuf::from("photo.dng")).unwrap().to_string(),
            "image/x-adobe-dng"
        );

        // Ensure all are image type
        let raw_file = from_path(&PathBuf::from("photo.cr2")).unwrap();
        assert_eq!(raw_file.type_(), "image");
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
