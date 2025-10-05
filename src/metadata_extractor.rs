use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use exif::{In, Reader, Tag, Value};
use log::debug;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::mimetype_detector;

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
        } else if let Ok(file) = File::open(path) {
            let mut reader = BufReader::new(file);

            match Reader::new().read_from_container(&mut reader) {
                Ok(exif_reader) => {
                    Self::extract_basic_info(&exif_reader, &mut metadata, file_metadata);
                    Self::extract_camera_info(&exif_reader, &mut metadata);
                    Self::extract_gps_info(&exif_reader, &mut metadata);
                }
                Err(e) => {
                    debug!("Failed to read EXIF data for {}: {}", path.display(), e);
                    // Even without EXIF, try file creation date fallback
                    Self::apply_file_creation_fallback(&mut metadata, file_metadata);
                }
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
        // Using functional style to find the first valid date
        metadata.taken_at = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
            .iter()
            .filter_map(|tag| reader.get_field(*tag, In::PRIMARY))
            .filter_map(|field| Self::parse_exif_datetime(&field.display_value().to_string()))
            .next(); // Take the first valid date found

        // If no EXIF date found, try GPS date as fallback
        if metadata.taken_at.is_none() {
            metadata.taken_at = Self::get_gps_date(reader);
        }

        // If still no date found, try file modification/creation date as final fallback
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
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.color_space = Some(value);
            }
        }

        if let Some(field) = reader.get_field(Tag::WhiteBalance, In::PRIMARY) {
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.white_balance = Some(value);
            }
        }

        if let Some(field) = reader.get_field(Tag::ExposureMode, In::PRIMARY) {
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.exposure_mode = Some(value);
            }
        }

        if let Some(field) = reader.get_field(Tag::MeteringMode, In::PRIMARY) {
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.metering_mode = Some(value);
            }
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
            // Prefer modification time over creation time as it's more reliable
            // Creation time on Linux shows when the file was copied to this filesystem,
            // not when the photo was actually taken
            if let Ok(modified_time) = fs_metadata.modified() {
                metadata.taken_at = Some(DateTime::from(modified_time));
            } else if let Ok(created_time) = fs_metadata.created() {
                metadata.taken_at = Some(DateTime::from(created_time));
            }
        }
    }

    fn extract_camera_info(reader: &exif::Exif, metadata: &mut PhotoMetadata) {
        if let Some(field) = reader.get_field(Tag::Make, In::PRIMARY) {
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.camera_make = Some(value);
            }
        }

        if let Some(field) = reader.get_field(Tag::Model, In::PRIMARY) {
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.camera_model = Some(value);
            }
        }

        if let Some(field) = reader.get_field(Tag::LensMake, In::PRIMARY) {
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.lens_make = Some(value);
            }
        }

        if let Some(field) = reader.get_field(Tag::LensModel, In::PRIMARY) {
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.lens_model = Some(value);
            }
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
            let value = Self::clean_exif_string(field.display_value().to_string());
            if !value.is_empty() {
                metadata.shutter_speed = Some(value);
            }
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
                NaiveDate::parse_from_str(&gps_date.display_value().to_string(), "%F").ok()
            })
            .and_then(|gps_date| gps_date.and_hms_opt(0, 0, 0))
            .map(|naive_dt| DateTime::from_naive_utc_and_offset(naive_dt, Utc))
    }

    fn extract_video_metadata(path: &Path, metadata: &mut PhotoMetadata) {
        // Basic video metadata extraction
        // Note: Video metadata extraction requires ffmpeg integration
        // For now, we set basic defaults and detect video format

        let mime_type = mimetype_detector::from_path(path);
        if let Some(mime) = mime_type {
            match mime.subtype() {
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
        metadata.duration = None; // Requires ffmpeg integration
        metadata.bitrate = None; // Requires ffmpeg integration
        metadata.frame_rate = None; // TODO: Extract actual frame rate

        // For videos, try to get date from file metadata
        // Prefer modification time over creation time as it's more reliable
        if let Ok(file_metadata) = std::fs::metadata(path) {
            if let Ok(modified_time) = file_metadata.modified() {
                metadata.taken_at = Some(DateTime::from(modified_time));
            } else if let Ok(created_time) = file_metadata.created() {
                metadata.taken_at = Some(DateTime::from(created_time));
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
    fn test_clean_exif_string_removes_quotes() {
        assert_eq!(
            MetadataExtractor::clean_exif_string("\"Canon\"".to_string()),
            "Canon"
        );
    }

    #[test]
    fn test_clean_exif_string_removes_null_bytes() {
        assert_eq!(
            MetadataExtractor::clean_exif_string("Canon\0\0\0".to_string()),
            "Canon"
        );
    }

    #[test]
    fn test_clean_exif_string_handles_arrays() {
        // Simulates array-like EXIF values with commas and empty strings
        assert_eq!(
            MetadataExtractor::clean_exif_string(
                "EF-S18-55mm f/3.5-5.6 IS\", \"\", \"\", \"\"".to_string()
            ),
            "EF-S18-55mm f/3.5-5.6 IS"
        );
    }

    #[test]
    fn test_clean_exif_string_trims_whitespace() {
        assert_eq!(
            MetadataExtractor::clean_exif_string("  Canon  ".to_string()),
            "Canon"
        );
    }

    #[test]
    fn test_clean_exif_string_empty_input() {
        assert_eq!(
            MetadataExtractor::clean_exif_string("\"\"".to_string()),
            ""
        );
    }

    #[test]
    fn test_clean_exif_string_complex_case() {
        // Complex case with quotes, null bytes, and array-like structure
        assert_eq!(
            MetadataExtractor::clean_exif_string(
                "\"EF-S18-55mm f/3.5-5.6 IS\0\0\", \"\", \"\", \"\"".to_string()
            ),
            "EF-S18-55mm f/3.5-5.6 IS"
        );
    }
}
