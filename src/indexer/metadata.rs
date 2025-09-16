use chrono::{DateTime, NaiveDateTime, Utc};
use exif::{In, Reader, Tag, Value};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tracing::{debug, warn};

pub struct MetadataExtractor;

impl MetadataExtractor {
    pub fn extract(path: &Path) -> PhotoMetadata {
        let mut metadata = PhotoMetadata::default();

        if let Ok(file) = File::open(path) {
            let mut reader = BufReader::new(file);

            if let Ok(exif_reader) = Reader::new().read_from_container(&mut reader) {
                Self::extract_basic_info(&exif_reader, &mut metadata);
                Self::extract_camera_info(&exif_reader, &mut metadata);
                Self::extract_gps_info(&exif_reader, &mut metadata);
            } else {
                debug!("No EXIF data found for: {}", path.display());
            }
        }

        metadata
    }

    fn extract_basic_info(reader: &exif::Exif, metadata: &mut PhotoMetadata) {
        if let Some(field) = reader.get_field(Tag::DateTime, In::PRIMARY) {
            if let Some(date_time) = Self::parse_exif_datetime(&field.display_value().to_string()) {
                metadata.date_taken = Some(date_time);
            }
        }

        if let Some(field) = reader.get_field(Tag::PixelXDimension, In::PRIMARY) {
            if let Value::Long(ref v) = field.value {
                if !v.is_empty() {
                    metadata.width = Some(v[0] as i32);
                }
            }
        }

        if let Some(field) = reader.get_field(Tag::PixelYDimension, In::PRIMARY) {
            if let Value::Long(ref v) = field.value {
                if !v.is_empty() {
                    metadata.height = Some(v[0] as i32);
                }
            }
        }

        if let Some(field) = reader.get_field(Tag::Orientation, In::PRIMARY) {
            if let Value::Short(ref v) = field.value {
                if !v.is_empty() {
                    metadata.orientation = Some(v[0] as i32);
                }
            }
        }
    }

    fn extract_camera_info(reader: &exif::Exif, metadata: &mut PhotoMetadata) {
        if let Some(field) = reader.get_field(Tag::Make, In::PRIMARY) {
            metadata.camera_make = Some(field.display_value().to_string());
        }

        if let Some(field) = reader.get_field(Tag::Model, In::PRIMARY) {
            metadata.camera_model = Some(field.display_value().to_string());
        }

        if let Some(field) = reader.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
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
            metadata.shutter_speed = Some(field.display_value().to_string());
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
        let lat = Self::get_gps_coordinate(reader, Tag::GPSLatitude, Tag::GPSLatitudeRef);
        let lon = Self::get_gps_coordinate(reader, Tag::GPSLongitude, Tag::GPSLongitudeRef);

        if let (Some(latitude), Some(longitude)) = (lat, lon) {
            metadata.gps_latitude = Some(latitude);
            metadata.gps_longitude = Some(longitude);
        }
    }

    fn get_gps_coordinate(reader: &exif::Exif, coord_tag: Tag, ref_tag: Tag) -> Option<f64> {
        let coord_field = reader.get_field(coord_tag, In::PRIMARY)?;
        let ref_field = reader.get_field(ref_tag, In::PRIMARY)?;

        if let Value::Rational(ref coords) = coord_field.value {
            if coords.len() >= 3 {
                let degrees = coords[0].to_f64();
                let minutes = coords[1].to_f64();
                let seconds = coords[2].to_f64();

                let mut decimal = degrees + minutes / 60.0 + seconds / 3600.0;

                let ref_str = ref_field.display_value().to_string();
                if ref_str == "S" || ref_str == "W" {
                    decimal = -decimal;
                }

                return Some(decimal);
            }
        }

        None
    }

    fn parse_exif_datetime(datetime_str: &str) -> Option<DateTime<Utc>> {
        let cleaned = datetime_str.replace("\"", "");

        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(&cleaned, "%Y-%m-%d %H:%M:%S") {
            Some(DateTime::from_naive_utc_and_offset(naive_dt, Utc))
        } else if let Ok(naive_dt) = NaiveDateTime::parse_from_str(&cleaned, "%Y:%m:%d %H:%M:%S") {
            Some(DateTime::from_naive_utc_and_offset(naive_dt, Utc))
        } else {
            warn!("Failed to parse datetime: {}", datetime_str);
            None
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct PhotoMetadata {
    pub date_taken: Option<DateTime<Utc>>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub orientation: Option<i32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_metadata_default() {
        let metadata = PhotoMetadata::default();

        assert!(metadata.date_taken.is_none());
        assert!(metadata.width.is_none());
        assert!(metadata.height.is_none());
        assert!(metadata.orientation.is_none());
        assert!(metadata.camera_make.is_none());
        assert!(metadata.camera_model.is_none());
        assert!(metadata.iso.is_none());
        assert!(metadata.aperture.is_none());
        assert!(metadata.shutter_speed.is_none());
        assert!(metadata.focal_length.is_none());
        assert!(metadata.gps_latitude.is_none());
        assert!(metadata.gps_longitude.is_none());
    }

    #[test]
    fn test_metadata_clone() {
        let mut metadata = PhotoMetadata::default();
        metadata.width = Some(1920);
        metadata.height = Some(1080);
        metadata.camera_make = Some("Canon".to_string());

        let cloned = metadata.clone();
        assert_eq!(cloned.width, Some(1920));
        assert_eq!(cloned.height, Some(1080));
        assert_eq!(cloned.camera_make, Some("Canon".to_string()));
    }

    #[test]
    fn test_extract_from_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let nonexistent_path = temp_dir.path().join("nonexistent.jpg");

        let metadata = MetadataExtractor::extract(&nonexistent_path);

        assert!(metadata.date_taken.is_none());
        assert!(metadata.width.is_none());
        assert!(metadata.camera_make.is_none());
    }

    #[test]
    fn test_extract_from_non_image_file() {
        let temp_dir = TempDir::new().unwrap();
        let text_file = temp_dir.path().join("test.txt");

        let mut file = File::create(&text_file).unwrap();
        file.write_all(b"This is not an image file").unwrap();

        let metadata = MetadataExtractor::extract(&text_file);

        assert!(metadata.date_taken.is_none());
        assert!(metadata.width.is_none());
        assert!(metadata.camera_make.is_none());
    }

    #[test]
    fn test_extract_from_invalid_image_file() {
        let temp_dir = TempDir::new().unwrap();
        let fake_image = temp_dir.path().join("fake.jpg");

        let mut file = File::create(&fake_image).unwrap();
        file.write_all(b"This is fake image data without EXIF")
            .unwrap();

        let metadata = MetadataExtractor::extract(&fake_image);

        assert!(metadata.date_taken.is_none());
        assert!(metadata.width.is_none());
        assert!(metadata.camera_make.is_none());
    }

    #[test]
    fn test_parse_exif_datetime_standard_format() {
        let result = MetadataExtractor::parse_exif_datetime("\"2023-12-25 14:30:45\"");
        assert!(result.is_some());

        let dt = result.unwrap();
        assert_eq!(dt.year(), 2023);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 25);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
        assert_eq!(dt.second(), 45);
    }

    #[test]
    fn test_parse_exif_datetime_colon_format() {
        let result = MetadataExtractor::parse_exif_datetime("\"2023:12:25 14:30:45\"");
        assert!(result.is_some());

        let dt = result.unwrap();
        assert_eq!(dt.year(), 2023);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 25);
    }

    #[test]
    fn test_parse_exif_datetime_no_quotes() {
        let result = MetadataExtractor::parse_exif_datetime("2023-12-25 14:30:45");
        assert!(result.is_some());

        let dt = result.unwrap();
        assert_eq!(dt.year(), 2023);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 25);
    }

    #[test]
    fn test_parse_exif_datetime_invalid() {
        let result = MetadataExtractor::parse_exif_datetime("invalid-date-format");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_exif_datetime_empty() {
        let result = MetadataExtractor::parse_exif_datetime("");
        assert!(result.is_none());
    }
}
