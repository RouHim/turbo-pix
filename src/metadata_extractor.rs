use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata;
use log::debug;
use std::path::Path;

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
            match Metadata::new_from_path(path) {
                Ok(exif_metadata) => {
                    Self::extract_basic_info(&exif_metadata, &mut metadata, file_metadata);
                    Self::extract_camera_info(&exif_metadata, &mut metadata);
                    Self::extract_gps_info(&exif_metadata, &mut metadata);
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
        exif: &Metadata,
        metadata: &mut PhotoMetadata,
        file_metadata: Option<&std::fs::Metadata>,
    ) {
        // Try multiple EXIF date tags in order of preference
        metadata.taken_at = Self::get_string_tag(exif, &ExifTag::DateTimeOriginal(String::new()))
            .and_then(|s| Self::parse_exif_datetime(&s))
            .or_else(|| {
                Self::get_string_tag(exif, &ExifTag::ModifyDate(String::new()))
                    .and_then(|s| Self::parse_exif_datetime(&s))
            });

        // If no EXIF date found, try GPS date as fallback
        if metadata.taken_at.is_none() {
            metadata.taken_at = Self::get_gps_date(exif);
        }

        // If still no date found, try file modification/creation date as final fallback
        if metadata.taken_at.is_none() {
            Self::apply_file_creation_fallback(metadata, file_metadata);
        }

        // ExifImageWidth/Height are the EXIF equivalents of PixelXDimension/PixelYDimension
        metadata.width = Self::get_u32_tag(exif, &ExifTag::ExifImageWidth(vec![]))
            .or_else(|| Self::get_u32_tag(exif, &ExifTag::ImageWidth(vec![])));
        metadata.height = Self::get_u32_tag(exif, &ExifTag::ExifImageHeight(vec![]))
            .or_else(|| Self::get_u32_tag(exif, &ExifTag::ImageHeight(vec![])));

        // Color and exposure settings (these are u16 enum values, convert to strings)
        metadata.color_space =
            Self::get_u16_tag(exif, &ExifTag::ColorSpace(vec![])).map(Self::color_space_to_string);
        metadata.white_balance = Self::get_u16_tag(exif, &ExifTag::WhiteBalance(vec![]))
            .map(Self::white_balance_to_string);
        metadata.exposure_mode = Self::get_u16_tag(exif, &ExifTag::ExposureMode(vec![]))
            .map(Self::exposure_mode_to_string);
        metadata.metering_mode = Self::get_u16_tag(exif, &ExifTag::MeteringMode(vec![]))
            .map(Self::metering_mode_to_string);

        // Orientation
        metadata.orientation =
            Self::get_u16_tag(exif, &ExifTag::Orientation(vec![])).map(|v| v as i32);

        // Flash
        metadata.flash_used = Self::get_u16_tag(exif, &ExifTag::Flash(vec![])).map(|v| v != 0);
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

    fn extract_camera_info(exif: &Metadata, metadata: &mut PhotoMetadata) {
        metadata.camera_make = Self::get_string_tag(exif, &ExifTag::Make(String::new()));
        metadata.camera_model = Self::get_string_tag(exif, &ExifTag::Model(String::new()));
        metadata.lens_make = Self::get_string_tag(exif, &ExifTag::LensMake(String::new()));
        metadata.lens_model = Self::get_string_tag(exif, &ExifTag::LensModel(String::new()));

        // ISO - try multiple tag variants for maximum compatibility:
        // - ISOSpeed (tag 0x8827): EXIF 2.3+ standard, stored as u32
        // - ISO (tag 0x8833): Legacy tag from older EXIF versions, stored as u16
        // We try ISOSpeed first as it's the modern standard, then fall back to ISO
        metadata.iso = Self::get_u32_tag(exif, &ExifTag::ISOSpeed(vec![]))
            .map(|v| v as i32)
            .or_else(|| Self::get_u16_tag(exif, &ExifTag::ISO(vec![])).map(|v| v as i32));

        // Aperture (F-number)
        metadata.aperture = Self::get_rational_tag(exif, &ExifTag::FNumber(vec![]));

        // Exposure time (shutter speed)
        metadata.shutter_speed =
            Self::get_rational_tag(exif, &ExifTag::ExposureTime(vec![])).map(|v| {
                if v >= 1.0 {
                    format!("{:.1} s", v)
                } else {
                    format!("1/{:.0}", 1.0 / v)
                }
            });

        // Focal length
        metadata.focal_length = Self::get_rational_tag(exif, &ExifTag::FocalLength(vec![]));
    }

    fn extract_gps_info(exif: &Metadata, metadata: &mut PhotoMetadata) {
        let lat_values = Self::get_rational_array_tag(exif, &ExifTag::GPSLatitude(vec![]));
        let lat_ref = Self::get_string_tag(exif, &ExifTag::GPSLatitudeRef(String::new()));
        let lon_values = Self::get_rational_array_tag(exif, &ExifTag::GPSLongitude(vec![]));
        let lon_ref = Self::get_string_tag(exif, &ExifTag::GPSLongitudeRef(String::new()));

        if let (Some(lat_vals), Some(lat_r)) = (lat_values, lat_ref) {
            // Validate GPS latitude components: degrees [0-90], minutes [0-60), seconds [0-60)
            if lat_vals.len() >= 3
                && lat_vals[0] <= 90.0
                && lat_vals[1] < 60.0
                && lat_vals[2] < 60.0
            {
                let lat = lat_vals[0] + lat_vals[1] / 60.0 + lat_vals[2] / 3600.0;
                // Final validation: latitude must be in range [-90, 90]
                if lat <= 90.0 {
                    metadata.latitude = Some(if lat_r.contains('S') { -lat } else { lat });
                } else {
                    debug!("Invalid GPS latitude value: {}", lat);
                }
            }
        }

        if let (Some(lon_vals), Some(lon_r)) = (lon_values, lon_ref) {
            // Validate GPS longitude components: degrees [0-180], minutes [0-60), seconds [0-60)
            if lon_vals.len() >= 3
                && lon_vals[0] <= 180.0
                && lon_vals[1] < 60.0
                && lon_vals[2] < 60.0
            {
                let lon = lon_vals[0] + lon_vals[1] / 60.0 + lon_vals[2] / 3600.0;
                // Final validation: longitude must be in range [-180, 180]
                if lon <= 180.0 {
                    metadata.longitude = Some(if lon_r.contains('W') { -lon } else { lon });
                } else {
                    debug!("Invalid GPS longitude value: {}", lon);
                }
            }
        }
    }

    fn get_gps_date(exif: &Metadata) -> Option<DateTime<Utc>> {
        Self::get_string_tag(exif, &ExifTag::GPSDateStamp(String::new()))
            .and_then(|date_str| NaiveDate::parse_from_str(&date_str, "%F").ok())
            .and_then(|gps_date| gps_date.and_hms_opt(0, 0, 0))
            .map(|naive_dt| DateTime::from_naive_utc_and_offset(naive_dt, Utc))
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

                        // Extract codec information from streams
                        if let Some(streams) = parsed["streams"].as_array() {
                            for stream in streams {
                                let codec_type = stream["codec_type"].as_str().unwrap_or("");

                                if codec_type == "video" {
                                    metadata.video_codec =
                                        stream["codec_name"].as_str().map(|s| s.to_string());

                                    if let Some(width) = stream["width"].as_i64() {
                                        metadata.width = Some(width as u32);
                                    }
                                    if let Some(height) = stream["height"].as_i64() {
                                        metadata.height = Some(height as u32);
                                    }

                                    // Extract frame rate
                                    if let Some(fps_str) = stream["r_frame_rate"].as_str() {
                                        // Frame rate is in format "num/den" (e.g., "30000/1001")
                                        if let Some((num, den)) = fps_str.split_once('/') {
                                            if let (Ok(n), Ok(d)) =
                                                (num.parse::<f64>(), den.parse::<f64>())
                                            {
                                                if d > 0.0 {
                                                    metadata.frame_rate = Some(n / d);
                                                }
                                            }
                                        }
                                    }
                                } else if codec_type == "audio" {
                                    metadata.audio_codec =
                                        stream["codec_name"].as_str().map(|s| s.to_string());
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                // Fallback to basic defaults if ffprobe fails
                debug!("ffprobe failed for {}, using defaults", path.display());
                metadata.video_codec = Some("unknown".to_string());
                metadata.audio_codec = Some("unknown".to_string());
            }
        }

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

    // Enum value to string conversion helpers
    // These convert EXIF numeric enum values to human-readable strings
    // Based on EXIF 2.3 specification

    /// Convert ColorSpace enum value to string
    /// 1 = sRGB, 2 = Adobe RGB, 65535 = Uncalibrated
    fn color_space_to_string(value: u16) -> String {
        match value {
            1 => "sRGB".to_string(),
            2 => "Adobe RGB".to_string(),
            65535 => "Uncalibrated".to_string(),
            _ => format!("Unknown ({})", value),
        }
    }

    /// Convert WhiteBalance enum value to string
    /// 0 = Auto, 1 = Manual
    fn white_balance_to_string(value: u16) -> String {
        match value {
            0 => "Auto".to_string(),
            1 => "Manual".to_string(),
            _ => format!("Unknown ({})", value),
        }
    }

    /// Convert ExposureMode enum value to string
    /// 0 = Auto, 1 = Manual, 2 = Auto bracket
    fn exposure_mode_to_string(value: u16) -> String {
        match value {
            0 => "Auto".to_string(),
            1 => "Manual".to_string(),
            2 => "Auto bracket".to_string(),
            _ => format!("Unknown ({})", value),
        }
    }

    /// Convert MeteringMode enum value to string
    /// Values: 0=Unknown, 1=Average, 2=Center-weighted, 3=Spot, 4=Multi-spot,
    ///         5=Multi-segment, 6=Partial, 255=Other
    fn metering_mode_to_string(value: u16) -> String {
        match value {
            0 => "Unknown".to_string(),
            1 => "Average".to_string(),
            2 => "Center-weighted average".to_string(),
            3 => "Spot".to_string(),
            4 => "Multi-spot".to_string(),
            5 => "Multi-segment".to_string(),
            6 => "Partial".to_string(),
            255 => "Other".to_string(),
            _ => format!("Unknown ({})", value),
        }
    }

    // Helper functions to extract tag values from little_exif Metadata
    // These provide a clean abstraction over little_exif's iterator-based API
    // Each function handles a specific data type and returns None for mismatches

    /// Extract a string value from an EXIF tag
    /// Cleans the value by removing null bytes, quotes, and trimming whitespace
    fn get_string_tag(exif: &Metadata, tag_template: &ExifTag) -> Option<String> {
        exif.get_tag(tag_template).next().and_then(|tag| match tag {
            // Camera and lens metadata
            ExifTag::Make(s)
            | ExifTag::Model(s)
            | ExifTag::LensMake(s)
            | ExifTag::LensModel(s)
            // Date/time tags
            | ExifTag::DateTimeOriginal(s)
            | ExifTag::ModifyDate(s)
            // GPS reference tags
            | ExifTag::GPSLatitudeRef(s)
            | ExifTag::GPSLongitudeRef(s)
            | ExifTag::GPSDateStamp(s) => {
                let cleaned = Self::clean_exif_string(s.clone());
                if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                }
            }
            _ => None,
        })
    }

    /// Extract a u16 value from an EXIF tag
    /// Used for small integer values and enum types
    fn get_u16_tag(exif: &Metadata, tag_template: &ExifTag) -> Option<u16> {
        exif.get_tag(tag_template).next().and_then(|tag| match tag {
            ExifTag::Orientation(v)
            | ExifTag::ISO(v)
            | ExifTag::Flash(v)
            | ExifTag::ColorSpace(v)
            | ExifTag::WhiteBalance(v)
            | ExifTag::ExposureMode(v)
            | ExifTag::MeteringMode(v) => v.first().copied(),
            _ => None,
        })
    }

    /// Extract a u32 value from an EXIF tag
    /// Used for image dimensions and larger integer values like ISOSpeed
    fn get_u32_tag(exif: &Metadata, tag_template: &ExifTag) -> Option<u32> {
        exif.get_tag(tag_template).next().and_then(|tag| match tag {
            ExifTag::ExifImageWidth(v)
            | ExifTag::ExifImageHeight(v)
            | ExifTag::ImageWidth(v)
            | ExifTag::ImageHeight(v)
            | ExifTag::ISOSpeed(v) => v.first().copied(),
            _ => None,
        })
    }

    /// Extract a rational number (fraction) from an EXIF tag and convert to f64
    /// Used for aperture, exposure time, focal length, etc.
    /// Converts uR64 (unsigned rational 64-bit) to floating point via nominator/denominator
    fn get_rational_tag(exif: &Metadata, tag_template: &ExifTag) -> Option<f64> {
        use little_exif::rational::uR64;
        exif.get_tag(tag_template).next().and_then(|tag| match tag {
            ExifTag::FNumber(v) | ExifTag::ExposureTime(v) | ExifTag::FocalLength(v) => v
                .first()
                .map(|r: &uR64| r.nominator as f64 / r.denominator as f64),
            _ => None,
        })
    }

    /// Extract an array of rational numbers from an EXIF tag
    /// Used for GPS coordinates (degrees, minutes, seconds)
    /// Each rational is converted to f64 via nominator/denominator division
    fn get_rational_array_tag(exif: &Metadata, tag_template: &ExifTag) -> Option<Vec<f64>> {
        use little_exif::rational::uR64;
        exif.get_tag(tag_template).next().and_then(|tag| match tag {
            ExifTag::GPSLatitude(v) | ExifTag::GPSLongitude(v) => Some(
                v.iter()
                    .map(|r: &uR64| r.nominator as f64 / r.denominator as f64)
                    .collect(),
            ),
            _ => None,
        })
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
        assert_eq!(MetadataExtractor::clean_exif_string("\"\"".to_string()), "");
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
