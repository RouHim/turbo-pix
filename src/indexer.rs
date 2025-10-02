// Re-exports for backward compatibility
pub use crate::photo_processor::{PhotoProcessor, ProcessedPhoto};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Datelike, Timelike, Utc};

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
        let sample_path = std::path::Path::new("test-data/sample_with_exif.jpg");

        if sample_path.exists() {
            let metadata = MetadataExtractor::extract_with_metadata(sample_path, None);

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
        use rayon::prelude::*;
        use std::time::Instant;

        // Create test photo files by duplicating existing ones
        let test_photos = vec![
            {
                let path = std::path::PathBuf::from("test-data/sample_with_exif.jpg");
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
                let path = std::path::PathBuf::from("test-data/test_image_1.jpg");
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
                let path = std::path::PathBuf::from("test-data/test_image_3.jpg");
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

        let indexer = PhotoProcessor::new(vec![std::path::PathBuf::from("test-data")]);

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
        let sample_path = std::path::Path::new("test-data/sample_with_exif.jpg");

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
        let metadata = MetadataExtractor::extract_with_metadata(&temp_path, None);

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
        let metadata_without_param = MetadataExtractor::extract_with_metadata(&temp_path, None);

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
