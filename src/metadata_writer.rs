use chrono::{DateTime, Utc};
use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata;
use little_exif::rational::uR64;
use std::path::Path;

/// Updates EXIF metadata in an image file
/// Only updates the specified fields, preserves all other EXIF data
pub fn update_metadata(
    file_path: &Path,
    taken_at: Option<DateTime<Utc>>,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> Result<(), String> {
    // Validate GPS coordinates
    if let Some(lat) = latitude {
        if !(-90.0..=90.0).contains(&lat) {
            return Err(format!(
                "Latitude out of range: {} (must be -90 to 90)",
                lat
            ));
        }
    }

    if let Some(lng) = longitude {
        if !(-180.0..=180.0).contains(&lng) {
            return Err(format!(
                "Longitude out of range: {} (must be -180 to 180)",
                lng
            ));
        }
    }

    // GPS coordinates must be provided together or not at all
    match (latitude, longitude) {
        (Some(_), None) => {
            return Err("Latitude provided without longitude".to_string());
        }
        (None, Some(_)) => {
            return Err("Longitude provided without latitude".to_string());
        }
        _ => {}
    }

    // Read existing metadata from file
    let mut metadata = Metadata::new_from_path(file_path)
        .map_err(|e| format!("Failed to read EXIF from file: {}", e))?;

    // Update taken_at if provided
    if let Some(dt) = taken_at {
        let datetime_str = dt.format("%Y:%m:%d %H:%M:%S").to_string();
        metadata.set_tag(ExifTag::DateTimeOriginal(datetime_str));
    }

    // Update GPS coordinates if provided
    if let (Some(lat), Some(lng)) = (latitude, longitude) {
        // Set latitude reference (N or S)
        let lat_ref = if lat >= 0.0 { "N" } else { "S" };
        metadata.set_tag(ExifTag::GPSLatitudeRef(lat_ref.to_string()));

        // Convert latitude to degrees, minutes, seconds
        let lat_abs = lat.abs();
        let lat_deg = lat_abs.floor();
        let lat_min = ((lat_abs - lat_deg) * 60.0).floor();
        let lat_sec = ((lat_abs - lat_deg) * 60.0 - lat_min) * 60.0;

        metadata.set_tag(ExifTag::GPSLatitude(vec![
            uR64 {
                nominator: lat_deg as u32,
                denominator: 1,
            },
            uR64 {
                nominator: lat_min as u32,
                denominator: 1,
            },
            uR64 {
                nominator: (lat_sec * 1000.0) as u32,
                denominator: 1000,
            },
        ]));

        // Set longitude reference (E or W)
        let lng_ref = if lng >= 0.0 { "E" } else { "W" };
        metadata.set_tag(ExifTag::GPSLongitudeRef(lng_ref.to_string()));

        // Convert longitude to degrees, minutes, seconds
        let lng_abs = lng.abs();
        let lng_deg = lng_abs.floor();
        let lng_min = ((lng_abs - lng_deg) * 60.0).floor();
        let lng_sec = ((lng_abs - lng_deg) * 60.0 - lng_min) * 60.0;

        metadata.set_tag(ExifTag::GPSLongitude(vec![
            uR64 {
                nominator: lng_deg as u32,
                denominator: 1,
            },
            uR64 {
                nominator: lng_min as u32,
                denominator: 1,
            },
            uR64 {
                nominator: (lng_sec * 1000.0) as u32,
                denominator: 1000,
            },
        ]));
    }

    // Write updated metadata back to file
    metadata
        .write_to_file(file_path)
        .map_err(|e| format!("Failed to write EXIF to file: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_image(temp_dir: &TempDir) -> std::path::PathBuf {
        // Copy a real test image with EXIF to temp directory
        let test_image = Path::new("test-data/sample_with_exif.jpg");
        let temp_image = temp_dir.path().join("test.jpg");

        fs::copy(test_image, &temp_image).expect("Failed to copy test image");

        temp_image
    }

    #[test]
    fn test_update_taken_at() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let new_date = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 0).unwrap();
        let result = update_metadata(&image_path, Some(new_date), None, None);

        assert!(result.is_ok(), "Failed to update taken_at: {:?}", result);

        // Verify the date was written
        let metadata = Metadata::new_from_path(&image_path).unwrap();
        let date_tag = metadata
            .get_tag(&ExifTag::DateTimeOriginal(String::new()))
            .next();

        assert!(date_tag.is_some(), "DateTimeOriginal tag not found");
        if let Some(ExifTag::DateTimeOriginal(s)) = date_tag {
            assert_eq!(s, "2024:03:15 14:30:00", "DateTimeOriginal value incorrect");
        }
    }

    #[test]
    fn test_update_gps_coordinates() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let result = update_metadata(&image_path, None, Some(40.7128), Some(-74.0060));

        assert!(result.is_ok(), "Failed to update GPS: {:?}", result);

        // Verify GPS tags were written
        let metadata = Metadata::new_from_path(&image_path).unwrap();

        let lat_ref = metadata
            .get_tag(&ExifTag::GPSLatitudeRef(String::new()))
            .next();
        let lng_ref = metadata
            .get_tag(&ExifTag::GPSLongitudeRef(String::new()))
            .next();

        assert!(lat_ref.is_some(), "GPSLatitudeRef tag not found");
        assert!(lng_ref.is_some(), "GPSLongitudeRef tag not found");

        if let Some(ExifTag::GPSLatitudeRef(s)) = lat_ref {
            assert_eq!(s, "N", "GPSLatitudeRef should be N");
        }
        if let Some(ExifTag::GPSLongitudeRef(s)) = lng_ref {
            assert_eq!(s, "W", "GPSLongitudeRef should be W");
        }
    }

    #[test]
    fn test_update_both_date_and_gps() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let new_date = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        let result = update_metadata(&image_path, Some(new_date), Some(51.5074), Some(-0.1278));

        assert!(result.is_ok(), "Failed to update both fields: {:?}", result);
    }

    #[test]
    fn test_validate_latitude_out_of_range() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let result = update_metadata(&image_path, None, Some(91.0), Some(0.0));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Latitude out of range"));

        let result = update_metadata(&image_path, None, Some(-91.0), Some(0.0));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_longitude_out_of_range() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let result = update_metadata(&image_path, None, Some(0.0), Some(181.0));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Longitude out of range"));

        let result = update_metadata(&image_path, None, Some(0.0), Some(-181.0));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_gps_must_be_paired() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let result = update_metadata(&image_path, None, Some(40.0), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("without longitude"));

        let result = update_metadata(&image_path, None, None, Some(-74.0));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("without latitude"));
    }

    #[test]
    fn test_negative_gps_coordinates() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        // Southern hemisphere, Western hemisphere
        let result = update_metadata(&image_path, None, Some(-33.8688), Some(-151.2093));
        assert!(result.is_ok());

        let metadata = Metadata::new_from_path(&image_path).unwrap();

        let lat_ref = metadata
            .get_tag(&ExifTag::GPSLatitudeRef(String::new()))
            .next();
        let lng_ref = metadata
            .get_tag(&ExifTag::GPSLongitudeRef(String::new()))
            .next();

        if let Some(ExifTag::GPSLatitudeRef(s)) = lat_ref {
            assert_eq!(s, "S", "Should set S for negative latitude");
        }
        if let Some(ExifTag::GPSLongitudeRef(s)) = lng_ref {
            assert_eq!(s, "W", "Should set W for negative longitude");
        }
    }
}
