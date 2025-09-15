use chrono::{DateTime, Utc};
use mime_guess::MimeGuess;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use tracing::{error, info};

use super::scanner::PhotoFile;
use super::{FileScanner, MetadataExtractor};
use crate::db::models::Photo;

pub struct PhotoProcessor {
    scanner: FileScanner,
}

impl PhotoProcessor {
    pub fn new(photo_paths: Vec<PathBuf>) -> Self {
        Self {
            scanner: FileScanner::new(photo_paths),
        }
    }

    pub fn process_all(&self) -> Vec<ProcessedPhoto> {
        let photo_files = self.scanner.scan();
        let mut processed_photos = Vec::new();

        info!("Processing {} photo files", photo_files.len());

        for photo_file in photo_files {
            match self.process_photo(&photo_file) {
                Ok(processed) => processed_photos.push(processed),
                Err(e) => {
                    error!("Failed to process {}: {}", photo_file.path.display(), e);
                }
            }
        }

        info!("Successfully processed {} photos", processed_photos.len());
        processed_photos
    }

    fn process_photo(
        &self,
        photo_file: &PhotoFile,
    ) -> Result<ProcessedPhoto, Box<dyn std::error::Error>> {
        let metadata = MetadataExtractor::extract(&photo_file.path);
        let hash = self.calculate_sha256(&photo_file.path)?;
        let mime_type = self.get_mime_type(&photo_file.path);

        let date_modified = photo_file
            .date_modified
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        Ok(ProcessedPhoto {
            path: photo_file.path.to_string_lossy().to_string(),
            filename: photo_file.filename.clone(),
            file_size: photo_file.file_size as i64,
            mime_type,
            date_taken: metadata.date_taken,
            date_modified: DateTime::from_timestamp(date_modified, 0).unwrap_or_else(Utc::now),
            width: metadata.width,
            height: metadata.height,
            orientation: metadata.orientation.unwrap_or(1),
            camera_make: metadata.camera_make,
            camera_model: metadata.camera_model,
            iso: metadata.iso,
            aperture: metadata.aperture,
            shutter_speed: metadata.shutter_speed,
            focal_length: metadata.focal_length,
            gps_latitude: metadata.gps_latitude,
            gps_longitude: metadata.gps_longitude,
            hash_sha256: hash,
        })
    }

    fn calculate_sha256(&self, path: &Path) -> Result<String, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buffer = [0; 8192];

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        Ok(format!("{:x}", hasher.finalize()))
    }

    fn get_mime_type(&self, path: &Path) -> String {
        MimeGuess::from_path(path)
            .first_or_octet_stream()
            .to_string()
    }
}

#[derive(Debug, Clone)]
pub struct ProcessedPhoto {
    pub path: String,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: String,
    pub date_taken: Option<DateTime<Utc>>,
    pub date_modified: DateTime<Utc>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub orientation: i32,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub hash_sha256: String,
}

impl From<ProcessedPhoto> for Photo {
    fn from(processed: ProcessedPhoto) -> Self {
        Photo {
            id: None,
            path: processed.path,
            filename: processed.filename,
            file_size: processed.file_size,
            mime_type: processed.mime_type,
            date_taken: processed.date_taken,
            date_modified: processed.date_modified,
            date_indexed: Utc::now(),
            width: processed.width,
            height: processed.height,
            orientation: processed.orientation,
            camera_make: processed.camera_make,
            camera_model: processed.camera_model,
            iso: processed.iso,
            aperture: processed.aperture,
            shutter_speed: processed.shutter_speed,
            focal_length: processed.focal_length,
            gps_latitude: processed.gps_latitude,
            gps_longitude: processed.gps_longitude,
            location_name: None,
            hash_md5: None,
            hash_sha256: Some(processed.hash_sha256),
            thumbnail_path: None,
            has_thumbnail: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_image_file(dir: &Path, filename: &str, content: &[u8]) -> PathBuf {
        let file_path = dir.join(filename);
        let mut file = File::create(&file_path).unwrap();
        file.write_all(content).unwrap();
        file_path
    }

    #[test]
    fn test_photo_processor_new() {
        let temp_dir = TempDir::new().unwrap();
        let paths = vec![temp_dir.path().to_path_buf()];

        let processor = PhotoProcessor::new(paths);
        assert!(std::ptr::addr_of!(processor.scanner).is_aligned());
    }

    #[test]
    fn test_process_all_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let paths = vec![temp_dir.path().to_path_buf()];
        let processor = PhotoProcessor::new(paths);

        let processed = processor.process_all();
        assert!(processed.is_empty());
    }

    #[test]
    fn test_process_all_with_images() {
        let temp_dir = TempDir::new().unwrap();

        create_test_image_file(temp_dir.path(), "test1.jpg", b"fake jpeg content 1");
        create_test_image_file(temp_dir.path(), "test2.png", b"fake png content 2");
        create_test_image_file(temp_dir.path(), "not_image.txt", b"text file content");

        let paths = vec![temp_dir.path().to_path_buf()];
        let processor = PhotoProcessor::new(paths);

        let processed = processor.process_all();
        assert_eq!(processed.len(), 2);

        let filenames: Vec<&str> = processed.iter().map(|p| p.filename.as_str()).collect();
        assert!(filenames.contains(&"test1.jpg"));
        assert!(filenames.contains(&"test2.png"));
        assert!(!filenames.contains(&"not_image.txt"));
    }

    #[test]
    fn test_calculate_sha256() {
        let temp_dir = TempDir::new().unwrap();
        let test_content = b"test content for hashing";
        let file_path = create_test_image_file(temp_dir.path(), "test.jpg", test_content);

        let processor = PhotoProcessor::new(vec![]);
        let hash = processor.calculate_sha256(&file_path).unwrap();

        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

        let hash2 = processor.calculate_sha256(&file_path).unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_calculate_sha256_different_files() {
        let temp_dir = TempDir::new().unwrap();
        let file1 = create_test_image_file(temp_dir.path(), "test1.jpg", b"content 1");
        let file2 = create_test_image_file(temp_dir.path(), "test2.jpg", b"content 2");

        let processor = PhotoProcessor::new(vec![]);
        let hash1 = processor.calculate_sha256(&file1).unwrap();
        let hash2 = processor.calculate_sha256(&file2).unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_get_mime_type() {
        let processor = PhotoProcessor::new(vec![]);

        assert_eq!(processor.get_mime_type(Path::new("test.jpg")), "image/jpeg");
        assert_eq!(processor.get_mime_type(Path::new("test.png")), "image/png");
        assert_eq!(processor.get_mime_type(Path::new("test.gif")), "image/gif");
        assert_eq!(
            processor.get_mime_type(Path::new("test.tiff")),
            "image/tiff"
        );
        assert_eq!(
            processor.get_mime_type(Path::new("test.unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_processed_photo_clone() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_image_file(temp_dir.path(), "test.jpg", b"test content");

        let paths = vec![temp_dir.path().to_path_buf()];
        let processor = PhotoProcessor::new(paths);
        let processed = processor.process_all();

        assert_eq!(processed.len(), 1);
        let original = &processed[0];
        let cloned = original.clone();

        assert_eq!(original.filename, cloned.filename);
        assert_eq!(original.file_size, cloned.file_size);
        assert_eq!(original.hash_sha256, cloned.hash_sha256);
    }

    #[test]
    fn test_processed_photo_to_photo_conversion() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_image_file(temp_dir.path(), "test.jpg", b"test content");

        let paths = vec![temp_dir.path().to_path_buf()];
        let processor = PhotoProcessor::new(paths);
        let processed = processor.process_all();

        assert_eq!(processed.len(), 1);
        let processed_photo = &processed[0];
        let photo: Photo = processed_photo.clone().into();

        assert_eq!(photo.filename, processed_photo.filename);
        assert_eq!(photo.file_size, processed_photo.file_size);
        assert_eq!(photo.mime_type, processed_photo.mime_type);
        assert_eq!(photo.hash_sha256, Some(processed_photo.hash_sha256.clone()));
        assert_eq!(photo.orientation, processed_photo.orientation);
        assert!(photo.id.is_none());
        assert!(photo.hash_md5.is_none());
        assert_eq!(photo.has_thumbnail, false);
    }

    #[test]
    fn test_processed_photo_fields() {
        let temp_dir = TempDir::new().unwrap();
        create_test_image_file(
            temp_dir.path(),
            "detailed_test.jpg",
            b"detailed test content",
        );

        let paths = vec![temp_dir.path().to_path_buf()];
        let processor = PhotoProcessor::new(paths);
        let processed = processor.process_all();

        assert_eq!(processed.len(), 1);
        let photo = &processed[0];

        assert_eq!(photo.filename, "detailed_test.jpg");
        assert!(photo.path.ends_with("detailed_test.jpg"));
        assert_eq!(photo.file_size, 21);
        assert_eq!(photo.mime_type, "image/jpeg");
        assert_eq!(photo.orientation, 1);
        assert!(!photo.hash_sha256.is_empty());
        assert_eq!(photo.hash_sha256.len(), 64);
    }

    #[test]
    fn test_calculate_sha256_nonexistent_file() {
        let processor = PhotoProcessor::new(vec![]);
        let result = processor.calculate_sha256(Path::new("/nonexistent/file.jpg"));
        assert!(result.is_err());
    }
}
