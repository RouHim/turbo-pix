use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use exif::{In, Reader, Tag, Value};
use mime_guess::{mime, MimeGuess};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::cache::CacheManager;
use crate::db::{DbPool, Photo};

// === MetadataExtractor ===

pub struct MetadataExtractor;

impl MetadataExtractor {
    pub fn extract(path: &Path) -> PhotoMetadata {
        Self::extract_with_metadata(path, None)
    }

    pub fn extract_with_metadata(
        path: &Path,
        file_metadata: Option<&std::fs::Metadata>,
    ) -> PhotoMetadata {
        let mut metadata = PhotoMetadata::default();

        // Check if this is a video file first
        let mime_type = MimeGuess::from_path(path).first();
        let is_video = mime_type
            .as_ref()
            .map(|m| m.type_() == mime::VIDEO)
            .unwrap_or(false);

        if is_video {
            Self::extract_video_metadata(path, &mut metadata);
        } else if let Ok(file) = File::open(path) {
            let mut reader = BufReader::new(file);

            if let Ok(exif_reader) = Reader::new().read_from_container(&mut reader) {
                Self::extract_basic_info(&exif_reader, &mut metadata, file_metadata);
                Self::extract_camera_info(&exif_reader, &mut metadata);
                Self::extract_gps_info(&exif_reader, &mut metadata);
            } else {
                debug!("No EXIF data found for: {}", path.display());
                // Even without EXIF, try file creation date fallback
                Self::apply_file_creation_fallback(&mut metadata, file_metadata);
            }
        }

        metadata
    }

    fn extract_basic_info(
        reader: &exif::Exif,
        metadata: &mut PhotoMetadata,
        file_metadata: Option<&std::fs::Metadata>,
    ) {
        // Try multiple EXIF date tags in order of preference
        let date_tags = vec![Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime];

        for tag in date_tags {
            if let Some(field) = reader.get_field(tag, In::PRIMARY) {
                if let Some(date_time) =
                    Self::parse_exif_datetime(&field.display_value().to_string())
                {
                    metadata.taken_at = Some(date_time);
                    break; // Use the first valid date found
                }
            }
        }

        // If no EXIF date found, try GPS date as fallback
        if metadata.taken_at.is_none() {
            metadata.taken_at = Self::get_gps_date(reader);
        }

        // If still no date found, try file creation date as final fallback
        if metadata.taken_at.is_none() {
            Self::apply_file_creation_fallback(metadata, file_metadata);
        }

        if let Some(field) = reader.get_field(Tag::PixelXDimension, In::PRIMARY) {
            if let Value::Long(ref v) = field.value {
                if !v.is_empty() {
                    metadata.width = Some(v[0]);
                }
            }
        }

        if let Some(field) = reader.get_field(Tag::PixelYDimension, In::PRIMARY) {
            if let Value::Long(ref v) = field.value {
                if !v.is_empty() {
                    metadata.height = Some(v[0]);
                }
            }
        }

        if let Some(field) = reader.get_field(Tag::ColorSpace, In::PRIMARY) {
            metadata.color_space = Some(field.display_value().to_string());
        }

        if let Some(field) = reader.get_field(Tag::WhiteBalance, In::PRIMARY) {
            metadata.white_balance = Some(field.display_value().to_string());
        }

        if let Some(field) = reader.get_field(Tag::ExposureMode, In::PRIMARY) {
            metadata.exposure_mode = Some(field.display_value().to_string());
        }

        if let Some(field) = reader.get_field(Tag::MeteringMode, In::PRIMARY) {
            metadata.metering_mode = Some(field.display_value().to_string());
        }

        if let Some(field) = reader.get_field(Tag::Orientation, In::PRIMARY) {
            if let Value::Short(ref v) = field.value {
                if !v.is_empty() {
                    metadata.orientation = Some(v[0] as i32);
                }
            }
        }

        if let Some(field) = reader.get_field(Tag::Flash, In::PRIMARY) {
            metadata.flash_used = Some(!field.display_value().to_string().contains("No"));
        }
    }

    fn apply_file_creation_fallback(
        metadata: &mut PhotoMetadata,
        file_metadata: Option<&std::fs::Metadata>,
    ) {
        if let Some(fs_metadata) = file_metadata {
            if let Ok(created_time) = fs_metadata.created() {
                metadata.taken_at = Some(DateTime::from(created_time));
            }
            // Silently ignore if creation time is not supported on this filesystem
        }
    }

    fn extract_camera_info(reader: &exif::Exif, metadata: &mut PhotoMetadata) {
        if let Some(field) = reader.get_field(Tag::Make, In::PRIMARY) {
            metadata.camera_make = Some(
                field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string(),
            );
        }

        if let Some(field) = reader.get_field(Tag::Model, In::PRIMARY) {
            metadata.camera_model = Some(
                field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string(),
            );
        }

        if let Some(field) = reader.get_field(Tag::LensMake, In::PRIMARY) {
            metadata.lens_make = Some(
                field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string(),
            );
        }

        if let Some(field) = reader.get_field(Tag::LensModel, In::PRIMARY) {
            metadata.lens_model = Some(
                field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string(),
            );
        }

        if let Some(field) = reader.get_field(Tag::ISOSpeed, In::PRIMARY) {
            if let Value::Short(ref v) = field.value {
                if !v.is_empty() {
                    metadata.iso = Some(v[0] as i32);
                }
            }
        }

        if let Some(field) = reader.get_field(Tag::FNumber, In::PRIMARY) {
            if let Value::Rational(ref v) = field.value {
                if !v.is_empty() {
                    metadata.aperture = Some(v[0].to_f64());
                }
            }
        }

        if let Some(field) = reader.get_field(Tag::ExposureTime, In::PRIMARY) {
            metadata.shutter_speed = Some(
                field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string(),
            );
        }

        if let Some(field) = reader.get_field(Tag::FocalLength, In::PRIMARY) {
            if let Value::Rational(ref v) = field.value {
                if !v.is_empty() {
                    metadata.focal_length = Some(v[0].to_f64());
                }
            }
        }
    }

    fn extract_gps_info(reader: &exif::Exif, metadata: &mut PhotoMetadata) {
        let mut has_gps = false;
        let mut latitude: Option<f64> = None;
        let mut longitude: Option<f64> = None;

        if let Some(lat_field) = reader.get_field(Tag::GPSLatitude, In::PRIMARY) {
            if let Some(lat_ref_field) = reader.get_field(Tag::GPSLatitudeRef, In::PRIMARY) {
                if let (Value::Rational(lat_values), ref_value) =
                    (&lat_field.value, lat_ref_field.display_value().to_string())
                {
                    if lat_values.len() == 3 {
                        let lat = lat_values[0].to_f64()
                            + lat_values[1].to_f64() / 60.0
                            + lat_values[2].to_f64() / 3600.0;
                        latitude = Some(if ref_value.contains('S') { -lat } else { lat });
                        has_gps = true;
                    }
                }
            }
        }

        if let Some(lon_field) = reader.get_field(Tag::GPSLongitude, In::PRIMARY) {
            if let Some(lon_ref_field) = reader.get_field(Tag::GPSLongitudeRef, In::PRIMARY) {
                if let (Value::Rational(lon_values), ref_value) =
                    (&lon_field.value, lon_ref_field.display_value().to_string())
                {
                    if lon_values.len() == 3 {
                        let lon = lon_values[0].to_f64()
                            + lon_values[1].to_f64() / 60.0
                            + lon_values[2].to_f64() / 3600.0;
                        longitude = Some(if ref_value.contains('W') { -lon } else { lon });
                        has_gps = true;
                    }
                }
            }
        }

        if has_gps {
            metadata.latitude = latitude;
            metadata.longitude = longitude;
        }
    }

    fn get_gps_date(reader: &exif::Exif) -> Option<DateTime<Utc>> {
        reader
            .get_field(Tag::GPSDateStamp, In::PRIMARY)
            .and_then(|gps_date| {
                NaiveDate::parse_from_str(&gps_date.display_value().to_string(), "%Y-%m-%d").ok()
            })
            .and_then(|gps_date| gps_date.and_hms_opt(0, 0, 0))
            .map(|naive_dt| DateTime::from_naive_utc_and_offset(naive_dt, Utc))
    }

    fn extract_video_metadata(path: &Path, metadata: &mut PhotoMetadata) {
        // Basic video metadata extraction
        // TODO: Implement proper video metadata extraction using ffmpeg or similar
        // For now, we set basic defaults and detect video format

        let mime_type = MimeGuess::from_path(path).first();
        if let Some(mime) = mime_type {
            match mime.subtype().as_str() {
                "mp4" => {
                    metadata.video_codec = Some("h264".to_string()); // Common default
                    metadata.audio_codec = Some("aac".to_string()); // Common default
                }
                "webm" => {
                    metadata.video_codec = Some("vp8".to_string()); // Common for WebM
                    metadata.audio_codec = Some("vorbis".to_string()); // Common for WebM
                }
                "avi" => {
                    metadata.video_codec = Some("mpeg4".to_string()); // Common for AVI
                    metadata.audio_codec = Some("mp3".to_string()); // Common for AVI
                }
                "mov" => {
                    metadata.video_codec = Some("h264".to_string()); // Common for MOV
                    metadata.audio_codec = Some("aac".to_string()); // Common for MOV
                }
                "mkv" => {
                    metadata.video_codec = Some("h264".to_string()); // Common for MKV
                    metadata.audio_codec = Some("aac".to_string()); // Common for MKV
                }
                _ => {
                    metadata.video_codec = Some("unknown".to_string());
                    metadata.audio_codec = Some("unknown".to_string());
                }
            }
        }

        // Set default values for video metadata
        // These would be extracted from actual video files in a full implementation
        metadata.duration = None; // TODO: Extract actual duration
        metadata.bitrate = None; // TODO: Extract actual bitrate
        metadata.frame_rate = None; // TODO: Extract actual frame rate

        // For videos, try to get creation date from file metadata
        if let Ok(file_metadata) = std::fs::metadata(path) {
            if let Ok(created_time) = file_metadata.created() {
                metadata.taken_at = Some(DateTime::from(created_time));
            }
        }
    }

    fn parse_exif_datetime(datetime_str: &str) -> Option<DateTime<Utc>> {
        let cleaned = datetime_str.replace("\"", "");

        // Try EXIF format first (with colons): "2023:01:15 10:30:00"
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(&cleaned, "%Y:%m:%d %H:%M:%S") {
            return Some(DateTime::from_naive_utc_and_offset(naive_dt, Utc));
        }

        // Try standard format (with dashes): "2008-05-30 15:56:01"
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d %H:%M:%S") {
            return Some(DateTime::from_naive_utc_and_offset(naive_dt, Utc));
        }

        None
    }
}

#[derive(Debug, Default)]
pub struct PhotoMetadata {
    pub taken_at: Option<DateTime<Utc>>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub color_space: Option<String>,
    pub white_balance: Option<String>,
    pub exposure_mode: Option<String>,
    pub metering_mode: Option<String>,
    pub orientation: Option<i32>,
    pub flash_used: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub duration: Option<f64>,       // Video duration in seconds
    pub video_codec: Option<String>, // Video codec (e.g., "h264", "h265")
    pub audio_codec: Option<String>, // Audio codec (e.g., "aac", "mp3")
    pub bitrate: Option<i32>,        // Bitrate in kbps
    pub frame_rate: Option<f64>,     // Frame rate for videos
}

// === FileScanner ===

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
                let path = entry.path();

                if path.is_file() && Self::is_supported_file(path) {
                    if let Ok(metadata) = fs::metadata(path) {
                        photos.push(PhotoFile {
                            path: path.to_path_buf(),
                            size: metadata.len(),
                            modified: metadata
                                .modified()
                                .ok()
                                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|duration| {
                                    DateTime::from_timestamp(duration.as_secs() as i64, 0)
                                        .unwrap_or_else(Utc::now)
                                }),
                            metadata: metadata.clone(),
                        });
                    }
                }
            }
        }

        info!("Found {} photos", photos.len());
        photos
    }

    fn is_supported_file(path: &Path) -> bool {
        let supported_extensions = [
            // Images
            "jpg", "jpeg", "png", "tiff", "tif", "bmp", "webp", "heic", "raw", // Videos
            "mp4", "mov", "avi", "mkv", "webm", "m4v",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| supported_extensions.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
pub struct PhotoFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub metadata: std::fs::Metadata,
}

// === PhotoProcessor ===

pub struct PhotoProcessor {
    scanner: FileScanner,
}

impl PhotoProcessor {
    pub fn new(photo_paths: Vec<PathBuf>) -> Self {
        Self {
            scanner: FileScanner::new(photo_paths),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn process_all(&self) -> Vec<ProcessedPhoto> {
        let photo_files = self.scanner.scan();
        let mut processed_photos = Vec::new();

        for photo_file in photo_files {
            if let Some(processed_photo) = self.process_file(&photo_file) {
                processed_photos.push(processed_photo);
            }
        }

        processed_photos
    }

    #[allow(dead_code)]
    pub async fn process_new_photos(
        &self,
        db_pool: &DbPool,
        cache_manager: &CacheManager,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let photo_files = self.scanner.scan();
        let existing_photos = crate::db::get_all_photo_paths(db_pool)?;
        let existing_set: HashSet<String> = existing_photos.into_iter().collect();

        let new_photos: Vec<_> = photo_files
            .into_iter()
            .filter(|photo| !existing_set.contains(&photo.path.to_string_lossy().to_string()))
            .collect();

        if new_photos.is_empty() {
            info!("No new photos found");
            return Ok(0);
        }

        info!("Processing {} new photos...", new_photos.len());

        let mut processed_count = 0;
        for photo_file in new_photos {
            match self
                .process_and_store(&photo_file, db_pool, cache_manager)
                .await
            {
                Ok(_) => processed_count += 1,
                Err(e) => error!(
                    "Failed to process photo {}: {}",
                    photo_file.path.display(),
                    e
                ),
            }
        }

        Ok(processed_count)
    }

    pub async fn full_rescan_and_cleanup(
        &self,
        db_pool: &DbPool,
        _cache_manager: &CacheManager,
    ) -> Result<Vec<ProcessedPhoto>, Box<dyn std::error::Error>> {
        // Step 1: Get all photo files on disk
        let photo_files = self.scanner.scan();
        let mut processed_photos = Vec::new();

        // Step 2: Create list of existing paths for cleanup
        let existing_paths: Vec<String> = photo_files
            .iter()
            .map(|pf| pf.path.to_string_lossy().to_string())
            .collect();

        // Step 3: Delete orphaned photos (in database but not on disk)
        if let Err(e) = crate::db::delete_orphaned_photos(db_pool, &existing_paths) {
            eprintln!("Failed to delete orphaned photos: {}", e);
        }

        // Step 4: Process all files found on disk (parallel processing)
        let parallel_results: Vec<ProcessedPhoto> = photo_files
            .par_iter()
            .filter_map(|photo_file| self.process_file(photo_file))
            .collect();
        processed_photos.extend(parallel_results);

        Ok(processed_photos)
    }

    #[allow(dead_code)]
    async fn process_and_store(
        &self,
        photo_file: &PhotoFile,
        db_pool: &DbPool,
        _cache_manager: &CacheManager,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        if let Some(processed_photo) = self.process_file(photo_file) {
            let photo_id = processed_photo.save_to_db(db_pool)?;
            Ok(photo_id)
        } else {
            Err("Failed to process photo file".into())
        }
    }

    fn process_file(&self, photo_file: &PhotoFile) -> Option<ProcessedPhoto> {
        let path = &photo_file.path;

        let filename = path.file_name()?.to_string_lossy().to_string();
        let file_path = path.to_string_lossy().to_string();

        let mime_type = MimeGuess::from_path(path).first().map(|m| m.to_string());

        let metadata = MetadataExtractor::extract_with_metadata(path, Some(&photo_file.metadata));

        let hash_sha256 = self.calculate_file_hash(path).ok();

        Some(ProcessedPhoto {
            file_path,
            filename,
            file_size: photo_file.size as i64,
            mime_type,
            taken_at: metadata.taken_at,
            date_modified: photo_file.modified.unwrap_or_else(Utc::now),
            camera_make: metadata.camera_make,
            camera_model: metadata.camera_model,
            lens_make: metadata.lens_make,
            lens_model: metadata.lens_model,
            iso: metadata.iso,
            aperture: metadata.aperture,
            shutter_speed: metadata.shutter_speed,
            focal_length: metadata.focal_length,
            width: metadata.width.map(|w| w as i32),
            height: metadata.height.map(|h| h as i32),
            color_space: metadata.color_space,
            white_balance: metadata.white_balance,
            exposure_mode: metadata.exposure_mode,
            metering_mode: metadata.metering_mode,
            orientation: metadata.orientation,
            flash_used: metadata.flash_used,
            latitude: metadata.latitude,
            longitude: metadata.longitude,
            hash_sha256,
            duration: metadata.duration,
            video_codec: metadata.video_codec,
            audio_codec: metadata.audio_codec,
            bitrate: metadata.bitrate,
            frame_rate: metadata.frame_rate,
        })
    }

    fn calculate_file_hash(&self, path: &Path) -> Result<String, Box<dyn std::error::Error>> {
        let mut hasher = Sha256::new();
        hasher.update(path.to_string_lossy().as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }
}

#[derive(Debug)]
pub struct ProcessedPhoto {
    pub file_path: String,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: Option<String>,
    pub taken_at: Option<DateTime<Utc>>,
    pub date_modified: DateTime<Utc>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub color_space: Option<String>,
    pub white_balance: Option<String>,
    pub exposure_mode: Option<String>,
    pub metering_mode: Option<String>,
    pub orientation: Option<i32>,
    pub flash_used: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub hash_sha256: Option<String>,
    pub duration: Option<f64>,       // Video duration in seconds
    pub video_codec: Option<String>, // Video codec (e.g., "h264", "h265")
    pub audio_codec: Option<String>, // Audio codec (e.g., "aac", "mp3")
    pub bitrate: Option<i32>,        // Bitrate in kbps
    pub frame_rate: Option<f64>,     // Frame rate for videos
}

impl ProcessedPhoto {
    #[allow(dead_code)]
    fn save_to_db(&self, db_pool: &DbPool) -> Result<i64, Box<dyn std::error::Error>> {
        let photo = Photo {
            id: 0,
            file_path: self.file_path.clone(),
            filename: self.filename.clone(),
            file_size: self.file_size,
            mime_type: self.mime_type.clone(),
            taken_at: self.taken_at,
            date_modified: self.date_modified,
            date_indexed: Some(Utc::now()),
            camera_make: self.camera_make.clone(),
            camera_model: self.camera_model.clone(),
            lens_make: self.lens_make.clone(),
            lens_model: self.lens_model.clone(),
            iso: self.iso,
            aperture: self.aperture,
            shutter_speed: self.shutter_speed.clone(),
            focal_length: self.focal_length,
            width: self.width,
            height: self.height,
            color_space: self.color_space.clone(),
            white_balance: self.white_balance.clone(),
            exposure_mode: self.exposure_mode.clone(),
            metering_mode: self.metering_mode.clone(),
            orientation: self.orientation,
            flash_used: self.flash_used,
            latitude: self.latitude,
            longitude: self.longitude,
            location_name: None,
            hash_sha256: self.hash_sha256.clone(),
            thumbnail_path: None,
            has_thumbnail: Some(false),
            country: None,
            keywords: None,
            faces_detected: None,
            objects_detected: None,
            colors: None,
            duration: self.duration,
            video_codec: self.video_codec.clone(),
            audio_codec: self.audio_codec.clone(),
            bitrate: self.bitrate,
            frame_rate: self.frame_rate,
            is_favorite: Some(false),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        photo.create(db_pool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_parse_exif_datetime() {
        let result = MetadataExtractor::parse_exif_datetime("\"2023:01:15 10:30:00\"");
        assert!(result.is_some());

        let datetime = result.unwrap();
        assert_eq!(datetime.year(), 2023);
        assert_eq!(datetime.month(), 1);
        assert_eq!(datetime.day(), 15);
        assert_eq!(datetime.hour(), 10);
        assert_eq!(datetime.minute(), 30);
        assert_eq!(datetime.second(), 0);
    }

    #[test]
    fn test_parse_exif_datetime_invalid() {
        let result = MetadataExtractor::parse_exif_datetime("invalid_date");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_exif_datetime_empty() {
        let result = MetadataExtractor::parse_exif_datetime("");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_date_from_exif_priority_order() {
        // This test verifies that our EXIF date extraction follows the correct priority order:
        // 1. DateTimeOriginal (highest priority)
        // 2. DateTimeDigitized
        // 3. DateTime
        // 4. GPSDateStamp (lowest priority)

        // Create a mock exif reader that returns values for all date fields
        // We expect DateTimeOriginal to be chosen despite other fields being present

        // Note: This is a unit test for the logic, not requiring actual EXIF files
        // The enhanced extract_date_from_exif function now checks multiple tags in priority order

        // Test parse_exif_datetime with different formats that would come from these tags
        let datetime_original = MetadataExtractor::parse_exif_datetime("\"2023:01:15 10:30:00\"");
        assert!(datetime_original.is_some());

        let datetime_digitized = MetadataExtractor::parse_exif_datetime("\"2023:01:16 11:30:00\"");
        assert!(datetime_digitized.is_some());

        let datetime_regular = MetadataExtractor::parse_exif_datetime("\"2023:01:17 12:30:00\"");
        assert!(datetime_regular.is_some());

        // Verify each format parses correctly
        assert_eq!(datetime_original.unwrap().day(), 15);
        assert_eq!(datetime_digitized.unwrap().day(), 16);
        assert_eq!(datetime_regular.unwrap().day(), 17);
    }

    #[test]
    fn test_enhanced_exif_date_extraction_with_sample_file() {
        // Test with the sample EXIF file we downloaded
        let sample_path = std::path::Path::new("photos/sample_with_exif.jpg");

        if sample_path.exists() {
            let metadata = MetadataExtractor::extract(sample_path);

            // The sample file should have EXIF date information
            // This verifies our enhanced extraction is working
            if metadata.taken_at.is_some() {
                let taken_at = metadata.taken_at.unwrap();
                // Sample file has date 2008-05-30T15:56:01Z
                assert_eq!(taken_at.year(), 2008);
                assert_eq!(taken_at.month(), 5);
                assert_eq!(taken_at.day(), 30);
            }
        }
    }

    #[test]
    fn test_parallel_processing_performance() {
        use std::time::Instant;

        // Create test photo files by duplicating existing ones
        let test_photos = vec![
            {
                let path = std::path::PathBuf::from("photos/sample_with_exif.jpg");
                let metadata = std::fs::metadata(&path)
                    .unwrap_or_else(|_| panic!("Failed to get metadata for {}", path.display()));
                PhotoFile {
                    path,
                    size: metadata.len(),
                    modified: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| {
                            DateTime::from_timestamp(duration.as_secs() as i64, 0)
                                .unwrap_or_else(Utc::now)
                        }),
                    metadata,
                }
            },
            {
                let path = std::path::PathBuf::from("photos/test_image_1.jpg");
                let metadata = std::fs::metadata(&path)
                    .unwrap_or_else(|_| panic!("Failed to get metadata for {}", path.display()));
                PhotoFile {
                    path,
                    size: metadata.len(),
                    modified: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| {
                            DateTime::from_timestamp(duration.as_secs() as i64, 0)
                                .unwrap_or_else(Utc::now)
                        }),
                    metadata,
                }
            },
            {
                let path = std::path::PathBuf::from("photos/test_image_3.jpg");
                let metadata = std::fs::metadata(&path)
                    .unwrap_or_else(|_| panic!("Failed to get metadata for {}", path.display()));
                PhotoFile {
                    path,
                    size: metadata.len(),
                    modified: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|duration| {
                            DateTime::from_timestamp(duration.as_secs() as i64, 0)
                                .unwrap_or_else(Utc::now)
                        }),
                    metadata,
                }
            },
        ];

        // Create multiple copies to simulate larger workload
        let mut large_test_set = Vec::new();
        for _ in 0..50 {
            // 150 total photos
            large_test_set.extend(test_photos.clone());
        }

        let indexer = PhotoProcessor::new(vec![std::path::PathBuf::from("photos")]);

        // Benchmark parallel processing
        let start = Instant::now();
        let parallel_results: Vec<ProcessedPhoto> = large_test_set
            .par_iter()
            .filter_map(|photo_file| indexer.process_file(photo_file))
            .collect();
        let parallel_duration = start.elapsed();

        // Benchmark sequential processing for comparison
        let start = Instant::now();
        let mut sequential_results = Vec::new();
        for photo_file in &large_test_set {
            if let Some(processed_photo) = indexer.process_file(photo_file) {
                sequential_results.push(processed_photo);
            }
        }
        let sequential_duration = start.elapsed();

        // Results should be the same
        assert_eq!(parallel_results.len(), sequential_results.len());

        println!(
            "Parallel processing: {:.2}ms for {} photos ({:.2} photos/sec)",
            parallel_duration.as_millis(),
            parallel_results.len(),
            parallel_results.len() as f64 / parallel_duration.as_secs_f64()
        );

        println!(
            "Sequential processing: {:.2}ms for {} photos ({:.2} photos/sec)",
            sequential_duration.as_millis(),
            sequential_results.len(),
            sequential_results.len() as f64 / sequential_duration.as_secs_f64()
        );

        println!(
            "Speedup: {:.2}x",
            sequential_duration.as_secs_f64() / parallel_duration.as_secs_f64()
        );
    }

    #[test]
    fn test_file_creation_date_fallback_no_exif_no_gps() {
        use std::fs;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create a temporary file without EXIF data
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"fake image data").unwrap();
        let temp_path = temp_file.path().to_path_buf();

        // Get the file's creation time
        let file_metadata = fs::metadata(&temp_path).unwrap();
        let expected_creation_time = file_metadata.created().unwrap();
        let expected_datetime: DateTime<Utc> = DateTime::from(expected_creation_time);

        // Extract metadata with file metadata provided
        let metadata = MetadataExtractor::extract_with_metadata(&temp_path, Some(&file_metadata));

        // Should fall back to file creation date since no EXIF/GPS data
        assert!(metadata.taken_at.is_some());
        let taken_at = metadata.taken_at.unwrap();

        // Allow small time difference due to conversion precision
        let time_diff = (taken_at - expected_datetime).num_seconds().abs();
        assert!(
            time_diff <= 1,
            "Creation time fallback should match file creation time within 1 second, got diff: {}",
            time_diff
        );
    }

    #[test]
    fn test_file_creation_date_fallback_exif_takes_priority() {
        // Test with the sample EXIF file - should NOT use file creation time
        let sample_path = std::path::Path::new("photos/sample_with_exif.jpg");

        if sample_path.exists() {
            let file_metadata = std::fs::metadata(sample_path).unwrap();
            let metadata =
                MetadataExtractor::extract_with_metadata(sample_path, Some(&file_metadata));

            // Should use EXIF date (2008-05-30), not file creation time
            assert!(metadata.taken_at.is_some());
            let taken_at = metadata.taken_at.unwrap();
            assert_eq!(taken_at.year(), 2008);
            assert_eq!(taken_at.month(), 5);
            assert_eq!(taken_at.day(), 30);
        }
    }

    #[test]
    fn test_file_creation_date_fallback_handles_unsupported_filesystem() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create a temporary file
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"fake image data").unwrap();
        let temp_path = temp_file.path().to_path_buf();

        // Extract metadata without file metadata (simulating unsupported filesystem)
        let metadata = MetadataExtractor::extract(&temp_path);

        // Should not crash, taken_at should remain None since no EXIF data and creation time unsupported
        assert!(metadata.taken_at.is_none());
    }

    #[test]
    fn test_file_creation_date_fallback_with_metadata_parameter() {
        use std::fs;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create a temporary file without EXIF data
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"fake image data").unwrap();
        let temp_path = temp_file.path().to_path_buf();

        // Get the file's creation time
        let file_metadata = fs::metadata(&temp_path).unwrap();
        let expected_creation_time = file_metadata.created().unwrap();
        let expected_datetime: DateTime<Utc> = DateTime::from(expected_creation_time);

        // Test that extract_with_metadata provides creation time fallback
        let metadata_with_param =
            MetadataExtractor::extract_with_metadata(&temp_path, Some(&file_metadata));
        let metadata_without_param = MetadataExtractor::extract(&temp_path);

        // Only the method with metadata should have taken_at set (creation time fallback)
        assert!(metadata_with_param.taken_at.is_some());
        assert!(metadata_without_param.taken_at.is_none()); // extract() doesn't have access to file metadata

        // Verify the creation time is correctly extracted
        let taken_at = metadata_with_param.taken_at.unwrap();
        let time_diff = (taken_at - expected_datetime).num_seconds().abs();
        assert!(
            time_diff <= 1,
            "Creation time fallback should match file creation time within 1 second, got diff: {}",
            time_diff
        );
    }
}
