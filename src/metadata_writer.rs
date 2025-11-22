use std::io::Cursor;
use std::path::Path;

use chrono::{DateTime, Utc};
use exif::{Field, In, Rational, Tag, Value};
use img_parts::jpeg::Jpeg;
use img_parts::png::Png;
use img_parts::{Bytes, ImageEXIF};

/// Updates EXIF metadata in an image file
/// Only updates the specified fields, preserves all other EXIF data and image content
///
/// Supported formats: JPEG (.jpg, .jpeg), PNG (.png)
/// Unsupported formats: WebP, RAW (CR2, CR3, NEF, ARW, DNG)
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

    // Determine format from file extension
    let extension = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .ok_or_else(|| "File has no extension".to_string())?;

    let format = match extension.as_str() {
        "jpg" | "jpeg" => "jpeg",
        "png" => "png",
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" => {
            return Err(format!(
                "RAW format '.{}' is not supported for EXIF writing. \
                Only JPEG and PNG are supported. \
                Convert to JPEG/PNG first or use specialized RAW editing tools.",
                extension
            ));
        }
        "webp" | "heic" | "heif" | "avif" => {
            return Err(format!(
                "Format '.{}' is not currently supported for EXIF writing. \
                Only JPEG and PNG are supported. Convert to JPEG/PNG first.",
                extension
            ));
        }
        _ => {
            return Err(format!(
                "Unsupported file extension '.{}' for EXIF writing. \
                Only JPEG (.jpg, .jpeg) and PNG (.png) are supported.",
                extension
            ));
        }
    };

    // Read existing EXIF data (or create empty if none exists)
    let file = std::fs::File::open(file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut bufreader = std::io::BufReader::new(&file);
    let exifreader = exif::Reader::new();

    let exif_result = exifreader.read_from_container(&mut bufreader);

    // Collect all fields we want to keep (excluding ones we're updating)
    let mut new_fields: Vec<Field> = Vec::new();

    // Copy existing fields if EXIF data exists, excluding ones we're updating
    if let Ok(exif) = exif_result {
        for field in exif.fields() {
            let should_keep = match field.tag {
                Tag::DateTimeOriginal if taken_at.is_some() => false,
                Tag::GPSLatitudeRef
                | Tag::GPSLatitude
                | Tag::GPSLongitudeRef
                | Tag::GPSLongitude
                    if latitude.is_some() && longitude.is_some() =>
                {
                    false
                }
                _ => true,
            };

            if should_keep {
                new_fields.push(Field {
                    tag: field.tag,
                    ifd_num: field.ifd_num,
                    value: field.value.clone(),
                });
            }
        }
    }
    // If no EXIF exists, we'll just create new fields below

    // Add updated taken_at if provided
    if let Some(dt) = taken_at {
        let datetime_str = dt.format("%Y:%m:%d %H:%M:%S").to_string();
        new_fields.push(Field {
            tag: Tag::DateTimeOriginal,
            ifd_num: In::PRIMARY,
            value: Value::Ascii(vec![datetime_str.as_bytes().to_vec()]),
        });
    }

    // Add updated GPS coordinates if provided
    if let (Some(lat), Some(lng)) = (latitude, longitude) {
        // Convert latitude to degrees, minutes, seconds
        let lat_abs = lat.abs();
        let lat_deg = lat_abs.floor();
        let lat_min = ((lat_abs - lat_deg) * 60.0).floor();
        let lat_sec = ((lat_abs - lat_deg) * 60.0 - lat_min) * 60.0;

        // Convert longitude to degrees, minutes, seconds
        let lng_abs = lng.abs();
        let lng_deg = lng_abs.floor();
        let lng_min = ((lng_abs - lng_deg) * 60.0).floor();
        let lng_sec = ((lng_abs - lng_deg) * 60.0 - lng_min) * 60.0;

        // Create GPS fields
        new_fields.push(Field {
            tag: Tag::GPSLatitudeRef,
            ifd_num: In::PRIMARY,
            value: Value::Ascii(vec![if lat >= 0.0 {
                b"N".to_vec()
            } else {
                b"S".to_vec()
            }]),
        });

        new_fields.push(Field {
            tag: Tag::GPSLatitude,
            ifd_num: In::PRIMARY,
            value: Value::Rational(vec![
                Rational {
                    num: lat_deg as u32,
                    denom: 1,
                },
                Rational {
                    num: lat_min as u32,
                    denom: 1,
                },
                Rational {
                    num: (lat_sec * 1000.0) as u32,
                    denom: 1000,
                },
            ]),
        });

        new_fields.push(Field {
            tag: Tag::GPSLongitudeRef,
            ifd_num: In::PRIMARY,
            value: Value::Ascii(vec![if lng >= 0.0 {
                b"E".to_vec()
            } else {
                b"W".to_vec()
            }]),
        });

        new_fields.push(Field {
            tag: Tag::GPSLongitude,
            ifd_num: In::PRIMARY,
            value: Value::Rational(vec![
                Rational {
                    num: lng_deg as u32,
                    denom: 1,
                },
                Rational {
                    num: lng_min as u32,
                    denom: 1,
                },
                Rational {
                    num: (lng_sec * 1000.0) as u32,
                    denom: 1000,
                },
            ]),
        });
    }

    // Generate new EXIF data using kamadak-exif Writer
    let mut exif_buffer = Cursor::new(Vec::new());
    let mut writer = exif::experimental::Writer::new();

    // Push all fields into the writer
    for field in &new_fields {
        writer.push_field(field);
    }

    // Write EXIF as TIFF to buffer (false = big-endian, standard EXIF format)
    writer
        .write(&mut exif_buffer, false)
        .map_err(|e| format!("Failed to generate EXIF data: {}", e))?;

    let exif_bytes = Bytes::from(exif_buffer.into_inner());

    // Handle different image formats based on detected format
    match format {
        "jpeg" => {
            let image_bytes =
                std::fs::read(file_path).map_err(|e| format!("Failed to read JPEG: {}", e))?;

            let mut jpeg = Jpeg::from_bytes(image_bytes.into())
                .map_err(|e| format!("Failed to parse JPEG: {}", e))?;

            // Set the new EXIF data (replaces APP1 segment while preserving image data)
            jpeg.set_exif(Some(exif_bytes));

            // Write the complete JPEG back to file
            let output_bytes = jpeg.encoder().bytes();
            std::fs::write(file_path, output_bytes)
                .map_err(|e| format!("Failed to write JPEG: {}", e))?;
        }
        "png" => {
            let image_bytes =
                std::fs::read(file_path).map_err(|e| format!("Failed to read PNG: {}", e))?;

            let mut png = Png::from_bytes(image_bytes.into())
                .map_err(|e| format!("Failed to parse PNG: {}", e))?;

            // Set the new EXIF data
            png.set_exif(Some(exif_bytes));

            // Write the complete PNG back to file
            let output_bytes = png.encoder().bytes();
            std::fs::write(file_path, output_bytes)
                .map_err(|e| format!("Failed to write PNG: {}", e))?;
        }
        _ => {
            // This should never happen due to earlier validation
            return Err(format!("Unsupported format: {}", format));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use image::GenericImageView;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_image(temp_dir: &TempDir) -> std::path::PathBuf {
        // Copy a real test image with EXIF to temp directory
        let test_image = Path::new("test-data/sample_with_exif.jpg");
        let temp_image = temp_dir.path().join("test.jpg");

        fs::copy(test_image, &temp_image).expect("Failed to copy test image");

        temp_image
    }

    fn read_exif(path: &Path) -> exif::Exif {
        let file = std::fs::File::open(path).unwrap();
        let mut bufreader = std::io::BufReader::new(&file);
        let exifreader = exif::Reader::new();
        exifreader.read_from_container(&mut bufreader).unwrap()
    }

    fn is_valid_jpeg(path: &Path) -> bool {
        // Check JPEG magic bytes
        let bytes = std::fs::read(path).unwrap();
        bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] == 0xD8
    }

    fn can_decode_image(path: &Path) -> bool {
        // Try to decode the image with the image crate
        image::open(path).is_ok()
    }

    #[test]
    fn test_update_taken_at() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let new_date = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 0).unwrap();
        let result = update_metadata(&image_path, Some(new_date), None, None);

        assert!(result.is_ok(), "Failed to update taken_at: {:?}", result);

        // Verify the file is still a valid JPEG
        assert!(
            is_valid_jpeg(&image_path),
            "File is no longer a valid JPEG!"
        );
        assert!(
            can_decode_image(&image_path),
            "Cannot decode image after metadata update!"
        );

        // Verify the date was written
        let exif = read_exif(&image_path);
        let date_field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);

        assert!(date_field.is_some(), "DateTimeOriginal tag not found");
        if let Some(field) = date_field {
            let value_str = field.display_value().to_string();
            // kamadak-exif may display dates with dashes or colons, both are valid
            assert!(
                value_str.contains("2024-03-15 14:30:00")
                    || value_str.contains("2024:03:15 14:30:00"),
                "DateTimeOriginal value incorrect: {}",
                value_str
            );
        }
    }

    #[test]
    fn test_update_gps_coordinates() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let result = update_metadata(&image_path, None, Some(40.7128), Some(-74.0060));

        assert!(result.is_ok(), "Failed to update GPS: {:?}", result);

        // Verify the file is still a valid JPEG
        assert!(
            is_valid_jpeg(&image_path),
            "File is no longer a valid JPEG!"
        );
        assert!(
            can_decode_image(&image_path),
            "Cannot decode image after metadata update!"
        );

        // Verify GPS tags were written
        let exif = read_exif(&image_path);

        let lat_ref = exif.get_field(Tag::GPSLatitudeRef, In::PRIMARY);
        let lng_ref = exif.get_field(Tag::GPSLongitudeRef, In::PRIMARY);

        assert!(lat_ref.is_some(), "GPSLatitudeRef tag not found");
        assert!(lng_ref.is_some(), "GPSLongitudeRef tag not found");

        if let Some(field) = lat_ref {
            let value_str = field.display_value().to_string();
            assert!(value_str.contains('N'), "GPSLatitudeRef should be N");
        }
        if let Some(field) = lng_ref {
            let value_str = field.display_value().to_string();
            assert!(value_str.contains('W'), "GPSLongitudeRef should be W");
        }
    }

    #[test]
    fn test_update_both_date_and_gps() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        let new_date = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        let result = update_metadata(&image_path, Some(new_date), Some(51.5074), Some(-0.1278));

        assert!(result.is_ok(), "Failed to update both fields: {:?}", result);

        // Verify the file is still a valid JPEG
        assert!(
            is_valid_jpeg(&image_path),
            "File is no longer a valid JPEG!"
        );
        assert!(
            can_decode_image(&image_path),
            "Cannot decode image after metadata update!"
        );
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

        // Verify the file is still a valid JPEG
        assert!(
            is_valid_jpeg(&image_path),
            "File is no longer a valid JPEG!"
        );

        let exif = read_exif(&image_path);

        let lat_ref = exif.get_field(Tag::GPSLatitudeRef, In::PRIMARY);
        let lng_ref = exif.get_field(Tag::GPSLongitudeRef, In::PRIMARY);

        if let Some(field) = lat_ref {
            let value_str = field.display_value().to_string();
            assert!(
                value_str.contains('S'),
                "Should set S for negative latitude"
            );
        }
        if let Some(field) = lng_ref {
            let value_str = field.display_value().to_string();
            assert!(
                value_str.contains('W'),
                "Should set W for negative longitude"
            );
        }
    }

    #[test]
    fn test_preserves_image_data() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        // Get original image data
        let original_size = std::fs::metadata(&image_path).unwrap().len();
        let original_image = image::open(&image_path).unwrap();
        let (orig_width, orig_height) = original_image.dimensions();

        // Update metadata
        let new_date = Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap();
        let result = update_metadata(&image_path, Some(new_date), Some(40.0), Some(-75.0));
        assert!(result.is_ok());

        // Verify image can still be decoded
        let updated_image = image::open(&image_path).unwrap();
        let (new_width, new_height) = updated_image.dimensions();

        // Dimensions must be identical
        assert_eq!(
            orig_width, new_width,
            "Image width changed after metadata update"
        );
        assert_eq!(
            orig_height, new_height,
            "Image height changed after metadata update"
        );

        // File size should be reasonable (JPEG segment changes can vary size slightly)
        let new_size = std::fs::metadata(&image_path).unwrap().len();
        let size_ratio = new_size as f64 / original_size as f64;
        assert!(
            (0.5..=1.5).contains(&size_ratio),
            "File size changed unreasonably: {} -> {} (ratio: {})",
            original_size,
            new_size,
            size_ratio
        );
    }

    #[test]
    fn test_pixel_perfect_preservation() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        // Load original image and capture ALL pixel data
        let original_image = image::open(&image_path).unwrap();
        let original_pixels: Vec<u8> = original_image.to_rgb8().into_raw();
        let (orig_width, orig_height) = original_image.dimensions();

        // Update metadata
        let new_date = Utc.with_ymd_and_hms(2024, 12, 25, 15, 30, 45).unwrap();
        let result = update_metadata(&image_path, Some(new_date), Some(51.5074), Some(-0.1278));
        assert!(result.is_ok(), "Metadata update failed: {:?}", result);

        // Load updated image and capture ALL pixel data
        let updated_image = image::open(&image_path).unwrap();
        let updated_pixels: Vec<u8> = updated_image.to_rgb8().into_raw();
        let (new_width, new_height) = updated_image.dimensions();

        // Verify dimensions haven't changed
        assert_eq!(
            orig_width, new_width,
            "Image width changed after metadata update"
        );
        assert_eq!(
            orig_height, new_height,
            "Image height changed after metadata update"
        );

        // CRITICAL: Verify every single pixel is IDENTICAL
        assert_eq!(
            original_pixels.len(),
            updated_pixels.len(),
            "Pixel buffer size changed"
        );

        // Compare pixel-by-pixel
        let mut differences = 0;
        for (i, (orig, updated)) in original_pixels
            .iter()
            .zip(updated_pixels.iter())
            .enumerate()
        {
            if orig != updated {
                differences += 1;
                if differences <= 10 {
                    // Log first 10 differences for debugging
                    eprintln!("Pixel difference at index {}: {} -> {}", i, orig, updated);
                }
            }
        }

        assert_eq!(
            differences, 0,
            "Found {} pixel differences! Image data was modified during metadata update. \
             This means JPEG was re-encoded, which is LOSSY and unacceptable.",
            differences
        );
    }

    #[test]
    fn test_write_to_image_without_exif() {
        // GIVEN: Image with no EXIF data (car.jpg)
        let temp_dir = TempDir::new().unwrap();
        let source_path = Path::new("test-data/car.jpg");
        if !source_path.exists() {
            return;
        }

        let image_path = temp_dir.path().join("test_no_exif.jpg");
        std::fs::copy(source_path, &image_path).unwrap();

        // WHEN: Write EXIF metadata to file without EXIF
        let new_date = Utc.with_ymd_and_hms(2024, 11, 11, 10, 30, 0).unwrap();
        let result = update_metadata(&image_path, Some(new_date), Some(48.8566), Some(2.3522));

        // THEN: Should succeed
        assert!(
            result.is_ok(),
            "Should be able to add EXIF to file without it: {:?}",
            result
        );

        // THEN: Verify written data can be read back
        let exif = read_exif(&image_path);

        // Check datetime
        let date_field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);
        assert!(date_field.is_some(), "DateTimeOriginal should be written");

        // Check GPS
        let lat_field = exif.get_field(Tag::GPSLatitude, In::PRIMARY);
        let lon_field = exif.get_field(Tag::GPSLongitude, In::PRIMARY);
        assert!(lat_field.is_some(), "GPS Latitude should be written");
        assert!(lon_field.is_some(), "GPS Longitude should be written");

        // THEN: Image should still be valid
        assert!(is_valid_jpeg(&image_path));
        assert!(can_decode_image(&image_path));
    }

    #[test]
    fn test_write_to_exif_without_datetime_gps() {
        // GIVEN: Image with EXIF (sample_with_exif.jpg) - we'll test updating it
        let temp_dir = TempDir::new().unwrap();
        let source_path = Path::new("test-data/sample_with_exif.jpg");
        if !source_path.exists() {
            return;
        }

        let image_path = temp_dir.path().join("test_partial_exif.jpg");
        std::fs::copy(source_path, &image_path).unwrap();

        // WHEN: Update datetime and GPS (file already has EXIF with camera info)
        let new_date = Utc.with_ymd_and_hms(2025, 1, 15, 14, 22, 30).unwrap();
        let result = update_metadata(&image_path, Some(new_date), Some(-33.8688), Some(151.2093));

        // THEN: Should succeed
        assert!(
            result.is_ok(),
            "Should update datetime/GPS in existing EXIF: {:?}",
            result
        );

        // THEN: Read back and verify
        let exif = read_exif(&image_path);

        let date_field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);
        assert!(date_field.is_some(), "DateTime should be updated in EXIF");

        let lat_field = exif.get_field(Tag::GPSLatitude, In::PRIMARY);
        assert!(lat_field.is_some(), "GPS should be added/updated in EXIF");

        // THEN: Camera info should be preserved
        let make_field = exif.get_field(Tag::Make, In::PRIMARY);
        assert!(make_field.is_some(), "Camera make should be preserved");

        // THEN: Image integrity
        assert!(is_valid_jpeg(&image_path));
        assert!(can_decode_image(&image_path));
    }

    #[test]
    fn test_read_write_cycle_with_complete_exif() {
        // GIVEN: Image with complete EXIF data (Canon EOS 40D)
        let temp_dir = TempDir::new().unwrap();
        let source_path = Path::new("test-data/sample_with_exif.jpg");
        if !source_path.exists() {
            return;
        }

        let image_path = temp_dir.path().join("test_complete.jpg");
        std::fs::copy(source_path, &image_path).unwrap();

        // WHEN: Read original EXIF
        let original_exif = read_exif(&image_path);
        let orig_make = original_exif.get_field(Tag::Make, In::PRIMARY);
        let _orig_model = original_exif.get_field(Tag::Model, In::PRIMARY);

        // WHEN: Update datetime and GPS
        let new_date = Utc.with_ymd_and_hms(2026, 6, 20, 8, 45, 12).unwrap();
        let result = update_metadata(&image_path, Some(new_date), Some(51.5074), Some(-0.1278));
        assert!(result.is_ok());

        // THEN: Read back and verify updates applied
        let updated_exif = read_exif(&image_path);

        let date_field = updated_exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);
        assert!(date_field.is_some(), "DateTime should be updated");

        let lat_field = updated_exif.get_field(Tag::GPSLatitude, In::PRIMARY);
        let lon_field = updated_exif.get_field(Tag::GPSLongitude, In::PRIMARY);
        assert!(lat_field.is_some(), "GPS Latitude should be updated");
        assert!(lon_field.is_some(), "GPS Longitude should be updated");

        // THEN: Other EXIF fields should be preserved
        let updated_make = updated_exif.get_field(Tag::Make, In::PRIMARY);
        let updated_model = updated_exif.get_field(Tag::Model, In::PRIMARY);
        assert!(updated_make.is_some(), "Camera make should be preserved");
        assert!(updated_model.is_some(), "Camera model should be preserved");

        if let (Some(orig), Some(updated)) = (orig_make, updated_make) {
            assert_eq!(
                orig.display_value().to_string(),
                updated.display_value().to_string(),
                "Camera make should be unchanged"
            );
        }

        // THEN: Image integrity
        assert!(is_valid_jpeg(&image_path));
        assert!(can_decode_image(&image_path));
    }

    #[test]
    fn test_partial_update_datetime_only() {
        // GIVEN: Image with EXIF
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        // WHEN: Update only datetime (no GPS)
        let new_date = Utc.with_ymd_and_hms(2027, 3, 10, 16, 20, 0).unwrap();
        let result = update_metadata(&image_path, Some(new_date), None, None);

        // THEN: Should succeed
        assert!(result.is_ok(), "Should update datetime without GPS");

        // THEN: Datetime updated, GPS should not exist
        let exif = read_exif(&image_path);
        let date_field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);
        assert!(date_field.is_some(), "DateTime should be written");

        // THEN: Image integrity
        assert!(is_valid_jpeg(&image_path));
        assert!(can_decode_image(&image_path));
    }

    #[test]
    fn test_partial_update_gps_only() {
        // GIVEN: Image with EXIF
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        // WHEN: Update only GPS (no datetime)
        let result = update_metadata(&image_path, None, Some(35.6762), Some(139.6503));

        // THEN: Should succeed
        assert!(result.is_ok(), "Should update GPS without datetime");

        // THEN: GPS updated
        let exif = read_exif(&image_path);
        let lat_field = exif.get_field(Tag::GPSLatitude, In::PRIMARY);
        let lon_field = exif.get_field(Tag::GPSLongitude, In::PRIMARY);
        assert!(lat_field.is_some(), "GPS should be written");
        assert!(lon_field.is_some(), "GPS should be written");

        // THEN: Image integrity
        assert!(is_valid_jpeg(&image_path));
        assert!(can_decode_image(&image_path));
    }

    #[test]
    fn test_multiple_write_cycles() {
        // GIVEN: Image with EXIF
        let temp_dir = TempDir::new().unwrap();
        let image_path = create_test_image(&temp_dir);

        // Capture original pixels
        let original_image = image::open(&image_path).unwrap();
        let original_pixels: Vec<u8> = original_image.to_rgb8().into_raw();

        // WHEN: Perform multiple update cycles
        for i in 0..3 {
            let new_date = Utc.with_ymd_and_hms(2024 + i, 1, 1, 12, 0, 0).unwrap();
            let result = update_metadata(
                &image_path,
                Some(new_date),
                Some(40.0 + i as f64),
                Some(-74.0),
            );
            assert!(result.is_ok(), "Update cycle {} should succeed", i);
        }

        // THEN: Final datetime should reflect last update
        let exif = read_exif(&image_path);
        let date_field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);
        assert!(date_field.is_some());

        // THEN: Pixels should still be IDENTICAL after multiple writes
        let final_image = image::open(&image_path).unwrap();
        let final_pixels: Vec<u8> = final_image.to_rgb8().into_raw();

        assert_eq!(
            original_pixels.len(),
            final_pixels.len(),
            "Pixel buffer size changed after multiple writes"
        );

        let differences: usize = original_pixels
            .iter()
            .zip(final_pixels.iter())
            .filter(|(a, b)| a != b)
            .count();

        assert_eq!(
            differences, 0,
            "Found {} pixel differences after {} write cycles! Image was re-encoded.",
            differences, 3
        );
    }

    #[test]
    fn test_overwrite_existing_datetime_gps() {
        // GIVEN: Image with EXIF containing datetime and GPS
        let temp_dir = TempDir::new().unwrap();
        let source_path = Path::new("test-data/sample_with_exif.jpg");
        if !source_path.exists() {
            return;
        }

        let image_path = temp_dir.path().join("test_overwrite.jpg");
        std::fs::copy(source_path, &image_path).unwrap();

        // WHEN: Write initial metadata
        let date1 = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        update_metadata(&image_path, Some(date1), Some(10.0), Some(20.0)).unwrap();

        // WHEN: Overwrite with new metadata
        let date2 = Utc.with_ymd_and_hms(2024, 12, 31, 23, 59, 59).unwrap();
        let result = update_metadata(&image_path, Some(date2), Some(50.0), Some(-100.0));
        assert!(result.is_ok(), "Should overwrite existing metadata");

        // THEN: Should contain the NEW values
        let exif = read_exif(&image_path);
        let date_field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);
        assert!(date_field.is_some());

        let date_str = date_field.unwrap().display_value().to_string();
        assert!(
            date_str.contains("2024-12-31") || date_str.contains("2024:12:31"),
            "Should contain new date, got: {}",
            date_str
        );

        // THEN: Image integrity
        assert!(is_valid_jpeg(&image_path));
        assert!(can_decode_image(&image_path));
    }

    #[test]
    fn test_reject_raw_formats() {
        // GIVEN: RAW format file (CR2)
        let path = Path::new("test-data/IMG_9899.CR2");
        if !path.exists() {
            return;
        }

        // WHEN: Attempt to write EXIF to RAW file
        let new_date = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        let result = update_metadata(path, Some(new_date), Some(40.0), Some(-74.0));

        // THEN: Should fail with clear error message
        assert!(result.is_err(), "Should reject RAW formats");
        let error = result.unwrap_err();
        assert!(
            error.contains("RAW format") || error.contains("not supported"),
            "Error should mention RAW format, got: {}",
            error
        );
        assert!(
            error.contains("JPEG") || error.contains("PNG"),
            "Error should suggest alternatives, got: {}",
            error
        );
    }

    #[test]
    fn test_format_detection() {
        // GIVEN: Various test files
        let test_cases = vec![
            ("test-data/sample_with_exif.jpg", true, "JPEG"),
            ("test-data/IMG_9899.CR2", false, "RAW"),
        ];

        for (file_path, should_succeed, format) in test_cases {
            let path = Path::new(file_path);
            if !path.exists() {
                continue;
            }

            // WHEN: Attempt to write EXIF
            let new_date = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
            let result = update_metadata(path, Some(new_date), None, None);

            // THEN: Check expected result
            assert_eq!(
                result.is_ok(),
                should_succeed,
                "Format {} should {} be supported: {:?}",
                format,
                if should_succeed { "" } else { "not" },
                result
            );
        }
    }

    #[test]
    fn test_png_without_exif() {
        // GIVEN: Create a simple PNG file without EXIF data
        let temp_dir = TempDir::new().unwrap();
        let png_path = temp_dir.path().join("test.png");

        // Create a simple 1x1 PNG using the image crate
        let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255u8, 0u8, 0u8]));
        img.save(&png_path).unwrap();

        // WHEN: Add EXIF metadata to PNG without EXIF
        let new_date = Utc.with_ymd_and_hms(2024, 11, 22, 12, 30, 0).unwrap();
        let result = update_metadata(&png_path, Some(new_date), Some(40.7128), Some(-74.0060));

        // THEN: Should succeed
        assert!(
            result.is_ok(),
            "Should add EXIF to PNG without EXIF: {:?}",
            result
        );

        // THEN: Verify EXIF was written
        let file = std::fs::File::open(&png_path).unwrap();
        let mut bufreader = std::io::BufReader::new(&file);
        let exifreader = exif::Reader::new();
        let exif = exifreader.read_from_container(&mut bufreader).unwrap();

        // Check datetime
        let date_field = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY);
        assert!(date_field.is_some(), "DateTimeOriginal should be written");

        // Check GPS
        let lat_field = exif.get_field(Tag::GPSLatitude, In::PRIMARY);
        let lon_field = exif.get_field(Tag::GPSLongitude, In::PRIMARY);
        assert!(lat_field.is_some(), "GPS Latitude should be written");
        assert!(lon_field.is_some(), "GPS Longitude should be written");

        // THEN: PNG should still be valid
        assert!(image::open(&png_path).is_ok(), "PNG should still be valid");
    }
}
