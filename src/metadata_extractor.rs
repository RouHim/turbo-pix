use std::path::Path;

use chrono::{DateTime, NaiveDateTime, Utc};
use exif::{In, Tag, Value};
use log::debug;

use crate::mimetype_detector;

/// Metadata extracted from EXIF/video files
/// This is the raw extraction layer - will be transformed into JSON for storage
#[derive(Debug, Default)]
pub struct PhotoMetadata {
    // Computational fields (used in logic)
    pub taken_at: Option<DateTime<Utc>>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<i32>,
    pub duration: Option<f64>, // Video duration in seconds

    // Camera metadata (stored in JSON)
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,

    // Settings metadata (stored in JSON)
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub color_space: Option<String>,
    pub white_balance: Option<String>,
    pub exposure_mode: Option<String>,
    pub metering_mode: Option<String>,
    pub flash_used: Option<bool>,

    // Location metadata (stored in JSON)
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,

    // Video metadata (stored in JSON)
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub bitrate: Option<i32>,
    pub frame_rate: Option<f64>,
}

pub struct MetadataExtractor;

impl MetadataExtractor {
    /// Clean EXIF string values by removing null bytes, trimming whitespace,
    /// and handling arrays with empty trailing values
    fn clean_exif_string(value: String) -> String {
        value
            .replace('\0', "") // Remove null bytes first
            .split(',') // Split by comma to handle array-like values
            .next() // Take first value
            .unwrap_or("")
            .trim() // Trim whitespace
            .trim_matches('"') // Remove surrounding quotes
            .trim() // Final trim in case quotes had spaces
            .to_string()
    }

    pub fn extract_with_metadata(
        path: &Path,
        file_metadata: Option<&std::fs::Metadata>,
    ) -> PhotoMetadata {
        let mut metadata = PhotoMetadata::default();

        // Check if this is a video file first
        let mime_type = mimetype_detector::from_path(path);
        let is_video = mime_type
            .as_ref()
            .map(|m| m.type_() == "video")
            .unwrap_or(false);

        if is_video {
            Self::extract_video_metadata(path, &mut metadata);
        } else {
            // Use kamadak-exif for all EXIF reading
            if let Err(e) = Self::extract_exif_metadata(path, &mut metadata, file_metadata) {
                debug!("Failed to read EXIF data for {}: {}", path.display(), e);
                // Even without EXIF, try file creation date fallback
                Self::apply_file_creation_fallback(&mut metadata, file_metadata);
            }
        }

        metadata
    }

    /// Extract all EXIF metadata using kamadak-exif
    fn extract_exif_metadata(
        path: &Path,
        metadata: &mut PhotoMetadata,
        file_metadata: Option<&std::fs::Metadata>,
    ) -> Result<(), String> {
        let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mut bufreader = std::io::BufReader::new(&file);
        let exifreader = exif::Reader::new();
        let exif = exifreader
            .read_from_container(&mut bufreader)
            .map_err(|e| format!("Failed to read EXIF: {}", e))?;

        Self::extract_basic_info(&exif, metadata);
        Self::extract_camera_info(&exif, metadata);
        Self::extract_gps_info(&exif, metadata);

        // If still no date found after EXIF extraction, try file modification/creation date as final fallback
        if metadata.taken_at.is_none() {
            Self::apply_file_creation_fallback(metadata, file_metadata);
        }

        Ok(())
    }

    fn extract_basic_info(exif: &exif::Exif, metadata: &mut PhotoMetadata) {
        // Extract datetime
        if let Some(field) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
            let datetime_str = field
                .display_value()
                .to_string()
                .trim_matches('"')
                .to_string();
            metadata.taken_at = Self::parse_exif_datetime(&datetime_str);
        }

        // Fallback to ModifyDate
        if metadata.taken_at.is_none() {
            if let Some(field) = exif.get_field(Tag::DateTime, In::PRIMARY) {
                let datetime_str = field
                    .display_value()
                    .to_string()
                    .trim_matches('"')
                    .to_string();
                metadata.taken_at = Self::parse_exif_datetime(&datetime_str);
            }
        }

        // Extract orientation
        if let Some(field) = exif.get_field(Tag::Orientation, In::PRIMARY) {
            if let Value::Short(ref vec) = field.value {
                if let Some(&val) = vec.first() {
                    metadata.orientation = Some(val as i32);
                }
            }
        }

        // Extract dimensions
        if let Some(field) = exif.get_field(Tag::PixelXDimension, In::PRIMARY) {
            if let Some(width) = field.value.get_uint(0) {
                metadata.width = Some(width);
            }
        }
        if let Some(field) = exif.get_field(Tag::PixelYDimension, In::PRIMARY) {
            if let Some(height) = field.value.get_uint(0) {
                metadata.height = Some(height);
            }
        }

        // If dimensions not found in EXIF tags, try ImageWidth/ImageLength
        if metadata.width.is_none() {
            if let Some(field) = exif.get_field(Tag::ImageWidth, In::PRIMARY) {
                if let Some(width) = field.value.get_uint(0) {
                    metadata.width = Some(width);
                }
            }
        }
        if metadata.height.is_none() {
            if let Some(field) = exif.get_field(Tag::ImageLength, In::PRIMARY) {
                if let Some(height) = field.value.get_uint(0) {
                    metadata.height = Some(height);
                }
            }
        }
    }

    fn extract_camera_info(exif: &exif::Exif, metadata: &mut PhotoMetadata) {
        // Camera make/model
        if let Some(field) = exif.get_field(Tag::Make, In::PRIMARY) {
            metadata.camera_make = Some(Self::clean_exif_string(field.display_value().to_string()));
        }
        if let Some(field) = exif.get_field(Tag::Model, In::PRIMARY) {
            metadata.camera_model =
                Some(Self::clean_exif_string(field.display_value().to_string()));
        }

        // Lens info
        if let Some(field) = exif.get_field(Tag::LensMake, In::PRIMARY) {
            metadata.lens_make = Some(Self::clean_exif_string(field.display_value().to_string()));
        }
        if let Some(field) = exif.get_field(Tag::LensModel, In::PRIMARY) {
            metadata.lens_model = Some(Self::clean_exif_string(field.display_value().to_string()));
        }

        // ISO
        if let Some(field) = exif.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
            if let Some(iso) = field.value.get_uint(0) {
                metadata.iso = Some(iso as i32);
            }
        }

        // Aperture
        if let Some(field) = exif.get_field(Tag::FNumber, In::PRIMARY) {
            if let Value::Rational(ref vec) = field.value {
                if let Some(rational) = vec.first() {
                    metadata.aperture = Some(rational.num as f64 / rational.denom as f64);
                }
            }
        }

        // Shutter speed
        if let Some(field) = exif.get_field(Tag::ExposureTime, In::PRIMARY) {
            metadata.shutter_speed = Some(field.display_value().to_string());
        }

        // Focal length
        if let Some(field) = exif.get_field(Tag::FocalLength, In::PRIMARY) {
            if let Value::Rational(ref vec) = field.value {
                if let Some(rational) = vec.first() {
                    metadata.focal_length = Some(rational.num as f64 / rational.denom as f64);
                }
            }
        }

        // Color space
        if let Some(field) = exif.get_field(Tag::ColorSpace, In::PRIMARY) {
            metadata.color_space = Some(field.display_value().to_string());
        }

        // White balance
        if let Some(field) = exif.get_field(Tag::WhiteBalance, In::PRIMARY) {
            metadata.white_balance = Some(field.display_value().to_string());
        }

        // Exposure mode
        if let Some(field) = exif.get_field(Tag::ExposureMode, In::PRIMARY) {
            metadata.exposure_mode = Some(field.display_value().to_string());
        }

        // Metering mode
        if let Some(field) = exif.get_field(Tag::MeteringMode, In::PRIMARY) {
            metadata.metering_mode = Some(field.display_value().to_string());
        }

        // Flash
        if let Some(field) = exif.get_field(Tag::Flash, In::PRIMARY) {
            if let Some(flash_val) = field.value.get_uint(0) {
                // Flash fired if bit 0 is set
                metadata.flash_used = Some((flash_val & 0x1) != 0);
            }
        }
    }

    fn extract_gps_info(exif: &exif::Exif, metadata: &mut PhotoMetadata) {
        // Extract GPS latitude
        if let Some(lat_field) = exif.get_field(Tag::GPSLatitude, In::PRIMARY) {
            if let Some(lat_ref_field) = exif.get_field(Tag::GPSLatitudeRef, In::PRIMARY) {
                if let Value::Rational(ref lat_vals) = lat_field.value {
                    if lat_vals.len() >= 3 {
                        let degrees = lat_vals[0].num as f64 / lat_vals[0].denom as f64;
                        let minutes = lat_vals[1].num as f64 / lat_vals[1].denom as f64;
                        let seconds = lat_vals[2].num as f64 / lat_vals[2].denom as f64;

                        let mut lat = degrees + minutes / 60.0 + seconds / 3600.0;

                        // Apply hemisphere
                        let lat_ref = lat_ref_field.display_value().to_string();
                        if lat_ref.contains('S') {
                            lat = -lat;
                        }

                        metadata.latitude = Some(lat);
                    }
                }
            }
        }

        // Extract GPS longitude
        if let Some(lon_field) = exif.get_field(Tag::GPSLongitude, In::PRIMARY) {
            if let Some(lon_ref_field) = exif.get_field(Tag::GPSLongitudeRef, In::PRIMARY) {
                if let Value::Rational(ref lon_vals) = lon_field.value {
                    if lon_vals.len() >= 3 {
                        let degrees = lon_vals[0].num as f64 / lon_vals[0].denom as f64;
                        let minutes = lon_vals[1].num as f64 / lon_vals[1].denom as f64;
                        let seconds = lon_vals[2].num as f64 / lon_vals[2].denom as f64;

                        let mut lon = degrees + minutes / 60.0 + seconds / 3600.0;

                        // Apply hemisphere
                        let lon_ref = lon_ref_field.display_value().to_string();
                        if lon_ref.contains('W') {
                            lon = -lon;
                        }

                        metadata.longitude = Some(lon);
                    }
                }
            }
        }
    }

    fn extract_video_metadata(path: &Path, metadata: &mut PhotoMetadata) {
        // Use ffprobe to extract actual video codec information
        let ffprobe_path = std::env::var("FFPROBE_PATH").unwrap_or_else(|_| "ffprobe".to_string());

        match std::process::Command::new(&ffprobe_path)
            .args([
                "-v",
                "quiet",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                path.to_str().unwrap(),
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                if let Ok(json_str) = String::from_utf8(output.stdout) {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        // Extract duration from format section
                        if let Some(duration_str) = parsed["format"]["duration"].as_str() {
                            metadata.duration = duration_str.parse::<f64>().ok();
                        }

                        // Extract bitrate from format section
                        if let Some(bitrate_str) = parsed["format"]["bit_rate"].as_str() {
                            metadata.bitrate = bitrate_str.parse::<i32>().ok();
                        }

                        // Extract video codec and frame rate from streams
                        if let Some(streams) = parsed["streams"].as_array() {
                            for stream in streams {
                                if stream["codec_type"].as_str() == Some("video") {
                                    metadata.video_codec =
                                        stream["codec_name"].as_str().map(String::from);

                                    // Parse frame rate
                                    if let Some(r_frame_rate) = stream["r_frame_rate"].as_str() {
                                        if let Some((num, denom)) = r_frame_rate.split_once('/') {
                                            if let (Ok(n), Ok(d)) =
                                                (num.parse::<f64>(), denom.parse::<f64>())
                                            {
                                                if d != 0.0 {
                                                    metadata.frame_rate = Some(n / d);
                                                }
                                            }
                                        }
                                    }

                                    // Extract dimensions
                                    if let Some(width) = stream["width"].as_u64() {
                                        metadata.width = Some(width as u32);
                                    }
                                    if let Some(height) = stream["height"].as_u64() {
                                        metadata.height = Some(height as u32);
                                    }
                                } else if stream["codec_type"].as_str() == Some("audio") {
                                    metadata.audio_codec =
                                        stream["codec_name"].as_str().map(String::from);
                                }
                            }
                        }
                    }
                }
            }
            Ok(_) => {
                debug!("ffprobe command failed for {}", path.display());
            }
            Err(e) => {
                debug!("Failed to run ffprobe for {}: {}", path.display(), e);
            }
        }
    }

    fn apply_file_creation_fallback(
        metadata: &mut PhotoMetadata,
        file_metadata: Option<&std::fs::Metadata>,
    ) {
        if metadata.taken_at.is_none() {
            if let Some(fs_metadata) = file_metadata {
                if let Ok(modified) = fs_metadata.modified() {
                    metadata.taken_at = Some(modified.into());
                }
            }
        }
    }

    pub fn parse_exif_datetime(datetime_str: &str) -> Option<DateTime<Utc>> {
        let cleaned = datetime_str.replace("\"", "");

        // Try EXIF format first (with colons): "2023:01:15 10:30:00"
        // Most cameras use this format per EXIF specification
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(&cleaned, "%Y:%m:%d %H:%M:%S") {
            return Some(DateTime::from_naive_utc_and_offset(naive_dt, Utc));
        }

        // Try standard ISO 8601 format (with dashes): "2023-01-15 10:30:00"
        // Some software normalizes dates to this format
        // %F is equivalent to %Y-%m-%d, %T is equivalent to %H:%M:%S
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(&cleaned, "%F %T") {
            return Some(DateTime::from_naive_utc_and_offset(naive_dt, Utc));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_canon_exif() {
        // GIVEN: Canon EOS 40D image with complete EXIF data
        let path = Path::new("test-data/sample_with_exif.jpg");
        if !path.exists() {
            return; // Skip if test file not available
        }

        // WHEN: Extract metadata
        let metadata = MetadataExtractor::extract_with_metadata(path, None);

        // THEN: Verify manufacturer and model
        assert_eq!(metadata.camera_make, Some("Canon".to_string()));
        assert_eq!(metadata.camera_model, Some("Canon EOS 1100D".to_string()));

        // THEN: Verify datetime extraction
        assert!(
            metadata.taken_at.is_some(),
            "Should extract DateTimeOriginal"
        );

        // THEN: Verify orientation
        assert!(metadata.orientation.is_some(), "Should extract orientation");
    }

    #[test]
    fn test_extract_nikon_exif() {
        // GIVEN: Nikon D80 image
        let path = Path::new("test-data/cat.jpg");
        if !path.exists() {
            return;
        }

        // WHEN: Extract metadata
        let metadata = MetadataExtractor::extract_with_metadata(path, None);

        // THEN: Verify Nikon manufacturer
        assert_eq!(metadata.camera_make, Some("NIKON CORPORATION".to_string()));
        assert_eq!(metadata.camera_model, Some("NIKON D80".to_string()));

        // THEN: Verify datetime
        assert!(metadata.taken_at.is_some());
    }

    #[test]
    fn test_extract_canon_with_lens() {
        // GIVEN: Canon EOS 1100D with lens information
        let path = Path::new("test-data/IMG_9377.jpg");
        if !path.exists() {
            return;
        }

        // WHEN: Extract metadata
        let metadata = MetadataExtractor::extract_with_metadata(path, None);

        // THEN: Verify camera
        assert_eq!(metadata.camera_make, Some("Canon".to_string()));
        assert_eq!(metadata.camera_model, Some("Canon EOS 1100D".to_string()));

        // THEN: Verify datetime extraction
        assert!(metadata.taken_at.is_some());

        // THEN: Should have dimensions
        assert!(metadata.width.is_some(), "Should extract width");
        assert!(metadata.height.is_some(), "Should extract height");
    }

    #[test]
    fn test_extract_raw_cr2() {
        // GIVEN: Canon CR2 RAW file
        let path = Path::new("test-data/IMG_9899.CR2");
        if !path.exists() {
            return;
        }

        // WHEN: Extract metadata (should work via rawloader)
        let metadata = MetadataExtractor::extract_with_metadata(path, None);

        // THEN: Verify camera
        assert_eq!(metadata.camera_make, Some("Canon".to_string()));
        assert_eq!(metadata.camera_model, Some("Canon EOS 1100D".to_string()));

        // THEN: Should have datetime
        assert!(metadata.taken_at.is_some());
    }

    #[test]
    fn test_image_without_exif() {
        // GIVEN: Image with no EXIF data
        let path = Path::new("test-data/car.jpg");
        if !path.exists() {
            return;
        }

        // WHEN: Extract metadata with file metadata for fallback
        let file_meta = std::fs::metadata(path).ok();
        let metadata = MetadataExtractor::extract_with_metadata(path, file_meta.as_ref());

        // THEN: Should fall back to file creation date
        assert!(
            metadata.taken_at.is_some(),
            "Should fall back to file timestamp when no EXIF datetime"
        );

        // THEN: Camera info should be None
        assert!(metadata.camera_make.is_none());
        assert!(metadata.camera_model.is_none());
    }

    #[test]
    fn test_corrupted_jpeg() {
        // GIVEN: Corrupted JPEG (only 13 bytes)
        let path = Path::new("test-data/test_image_1.jpg");
        if !path.exists() {
            return;
        }

        // WHEN: Extract metadata with file metadata for fallback
        let file_meta = std::fs::metadata(path).ok();
        let metadata = MetadataExtractor::extract_with_metadata(path, file_meta.as_ref());

        // THEN: Should not crash and should attempt file timestamp fallback
        assert!(
            metadata.taken_at.is_some(),
            "Should fall back to file timestamp for corrupted files"
        );
    }

    #[test]
    fn test_datetime_parsing_formats() {
        // GIVEN: Various EXIF datetime formats
        let test_cases = vec![
            ("2024:03:15 14:30:00", true),  // Standard EXIF format
            ("2024-03-15 14:30:00", true),  // ISO 8601 format
            ("2024:03:15T14:30:00", false), // Invalid (T separator not supported)
            ("invalid", false),             // Invalid
            ("", false),                    // Empty
        ];

        for (input, should_parse) in test_cases {
            // WHEN: Parse datetime
            let result = MetadataExtractor::parse_exif_datetime(input);

            // THEN: Check expected result
            assert_eq!(
                result.is_some(),
                should_parse,
                "Failed for input: '{}'",
                input
            );
        }
    }

    #[test]
    fn test_gps_coordinate_extraction() {
        // GIVEN: This tests the GPS extraction logic indirectly
        // GPS coordinates should be converted from degrees/minutes/seconds to decimal

        // Example: 43°28'2.81"N = 43 + 28/60 + 2.81/3600 ≈ 43.46745
        // Example: 11°53'6.46"E = 11 + 53/60 + 6.46/3600 ≈ 11.88513

        // This would require a test image with GPS data
        // Currently our test images don't have GPS, so we verify the structure works
        let path = Path::new("test-data/sample_with_exif.jpg");
        if !path.exists() {
            return;
        }

        let metadata = MetadataExtractor::extract_with_metadata(path, None);

        // THEN: GPS fields should exist (even if None)
        // This ensures the extraction logic doesn't panic
        let _lat = metadata.latitude;
        let _lon = metadata.longitude;
    }

    #[test]
    fn test_clean_exif_string() {
        // GIVEN: Various EXIF string formats
        let test_cases = vec![
            ("Canon", "Canon"),
            ("\"Canon\"", "Canon"),
            ("Canon\0", "Canon"),
            ("  Canon  ", "Canon"),
            ("Canon, EOS", "Canon"), // Takes first value before comma
            ("\"  Canon  \"", "Canon"),
        ];

        for (input, expected) in test_cases {
            // WHEN: Clean string
            let result = MetadataExtractor::clean_exif_string(input.to_string());

            // THEN: Should match expected
            assert_eq!(result, expected, "Failed for input: '{}'", input);
        }
    }

    #[test]
    fn test_video_metadata_extraction() {
        // GIVEN: MP4 video file
        let path = Path::new("test-data/test_video.mp4");
        if !path.exists() {
            return;
        }

        // WHEN: Extract metadata with file metadata for fallback
        let file_meta = std::fs::metadata(path).ok();
        let metadata = MetadataExtractor::extract_with_metadata(path, file_meta.as_ref());

        // THEN: Should extract width and height
        assert!(metadata.width.is_some(), "Should extract video width");
        assert!(metadata.height.is_some(), "Should extract video height");

        // NOTE: Video metadata extraction doesn't currently apply file timestamp fallback
        // so taken_at may be None for videos without creation time metadata
    }

    #[test]
    fn test_metadata_extractor_doesnt_panic_on_various_files() {
        // GIVEN: All test files
        let test_files = vec![
            "test-data/sample_with_exif.jpg",
            "test-data/cat.jpg",
            "test-data/car.jpg",
            "test-data/IMG_9377.jpg",
            "test-data/test_image_1.jpg",
            "test-data/test_image_3.jpg",
            "test-data/test_video.mp4",
        ];

        for file_path in test_files {
            let path = Path::new(file_path);
            if !path.exists() {
                continue;
            }

            // WHEN: Extract metadata (should never panic)
            let _metadata = MetadataExtractor::extract_with_metadata(path, None);

            // THEN: If we reach here, no panic occurred
        }
    }
}
