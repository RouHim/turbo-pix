use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};
use walkdir::WalkDir;

pub struct FileScanner {
    photo_paths: Vec<PathBuf>,
}

impl FileScanner {
    pub fn new(photo_paths: Vec<PathBuf>) -> Self {
        Self { photo_paths }
    }

    pub fn scan(&self) -> Vec<PhotoFile> {
        let mut photos = Vec::new();

        for root_path in &self.photo_paths {
            if !root_path.exists() {
                warn!("Photo directory does not exist: {}", root_path.display());
                continue;
            }

            info!("Scanning directory: {}", root_path.display());

            for entry in WalkDir::new(root_path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() {
                    if let Some(photo_file) = self.process_file(entry.path()) {
                        photos.push(photo_file);
                    }
                }
            }
        }

        info!("Found {} photo files", photos.len());
        photos
    }

    fn process_file(&self, path: &Path) -> Option<PhotoFile> {
        let extension = path.extension()?.to_str()?.to_lowercase();

        if !Self::is_supported_image(&extension) {
            return None;
        }

        let metadata = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) => {
                warn!("Failed to read metadata for {}: {}", path.display(), e);
                return None;
            }
        };

        let filename = path.file_name()?.to_str()?.to_string();
        let file_size = metadata.len();
        let date_modified = metadata.modified().ok()?;

        Some(PhotoFile {
            path: path.to_path_buf(),
            filename,
            file_size,
            date_modified,
            extension,
        })
    }

    fn is_supported_image(extension: &str) -> bool {
        matches!(
            extension,
            "jpg"
                | "jpeg"
                | "png"
                | "tiff"
                | "tif"
                | "bmp"
                | "webp"
                | "raw"
                | "cr2"
                | "nef"
                | "arw"
                | "dng"
                | "orf"
                | "rw2"
        )
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PhotoFile {
    pub path: PathBuf,
    pub filename: String,
    pub file_size: u64,
    pub date_modified: std::time::SystemTime,
    pub extension: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_is_supported_image() {
        assert!(FileScanner::is_supported_image("jpg"));
        assert!(FileScanner::is_supported_image("jpeg"));
        assert!(FileScanner::is_supported_image("png"));
        assert!(FileScanner::is_supported_image("raw"));
        assert!(!FileScanner::is_supported_image("txt"));
        assert!(!FileScanner::is_supported_image("pdf"));
    }

    #[test]
    fn test_scanner_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let scanner = FileScanner::new(vec![temp_dir.path().to_path_buf()]);
        let photos = scanner.scan();
        assert!(photos.is_empty());
    }

    #[test]
    fn test_scanner_with_photos() {
        let temp_dir = TempDir::new().unwrap();

        // Create a fake JPEG file
        let jpeg_path = temp_dir.path().join("test.jpg");
        let mut file = File::create(&jpeg_path).unwrap();
        file.write_all(b"fake jpeg content").unwrap();

        // Create a non-image file
        let txt_path = temp_dir.path().join("readme.txt");
        let mut file = File::create(&txt_path).unwrap();
        file.write_all(b"some text").unwrap();

        let scanner = FileScanner::new(vec![temp_dir.path().to_path_buf()]);
        let photos = scanner.scan();

        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].filename, "test.jpg");
        assert_eq!(photos[0].extension, "jpg");
    }
}
