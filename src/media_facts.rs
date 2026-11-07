//! File-derived capture facts.
//!
//! The photo file is the authority for its capture date and coordinates. This
//! module is the *reader*: it turns a path into [`MediaFacts`]. The database
//! keeps a derived copy of those values (`photos.taken_at`, `taken_latitude`,
//! `taken_longitude`) for ordering, filtering and map placement, written only
//! from a fresh read here.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::metadata_extractor::MetadataExtractor;

/// Capture facts read from a photo file.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaFacts {
    pub taken_at: Option<DateTime<Utc>>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Facts of `path`, with the indexing fallback order (embedded → filename →
/// file timestamp).
pub fn read_media_facts(path: &Path) -> MediaFacts {
    read_media_facts_with_metadata(path, std::fs::metadata(path).ok().as_ref())
}

/// Facts of `path`, with the indexing fallback order (embedded → filename →
/// file timestamp), using pre-fetched file metadata when given.
pub fn read_media_facts_with_metadata(
    path: &Path,
    file_metadata: Option<&std::fs::Metadata>,
) -> MediaFacts {
    let extracted = MetadataExtractor::extract_with_metadata(path, file_metadata);
    MediaFacts {
        taken_at: extracted.taken_at,
        latitude: extracted.latitude,
        longitude: extracted.longitude,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::TimeZone;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn copy_fixture(temp_dir: &TempDir) -> PathBuf {
        let source = Path::new("test-data/IMG_9377.jpg");
        assert!(source.exists(), "test fixture missing: {source:?}");
        let dest = temp_dir.path().join("photo.jpg");
        fs::copy(source, &dest).expect("failed to copy test fixture");
        dest
    }

    #[test]
    fn read_media_facts_round_trips_written_date_and_coordinates() {
        // GIVEN: A real JPEG fixture and a date/coordinate pair written into it
        let temp_dir = TempDir::new().unwrap();
        let path = copy_fixture(&temp_dir);
        let written = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 0).unwrap();
        crate::metadata_writer::update_metadata(
            &path,
            Some(written),
            Some(40.7128),
            Some(-74.0060),
        )
        .expect("failed to write metadata");

        // WHEN: Reading the facts back from the file
        let facts = read_media_facts(&path);

        // THEN: The instant round-trips exactly (UTC, second precision)
        assert_eq!(facts.taken_at, Some(written));
        // AND: Coordinates come back at the file's DMS precision
        let latitude = facts.latitude.expect("latitude missing");
        let longitude = facts.longitude.expect("longitude missing");
        assert!(
            (latitude - 40.7128).abs() < 1e-4,
            "latitude drifted: {latitude}"
        );
        assert!(
            (longitude - (-74.0060)).abs() < 1e-4,
            "longitude drifted: {longitude}"
        );
    }

    #[test]
    fn read_media_facts_does_not_drift_across_rewrites() {
        // GIVEN: A file whose metadata was written once
        let temp_dir = TempDir::new().unwrap();
        let path = copy_fixture(&temp_dir);
        let written = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 0).unwrap();
        crate::metadata_writer::update_metadata(
            &path,
            Some(written),
            Some(40.7128),
            Some(-74.0060),
        )
        .expect("failed to write metadata");
        let first = read_media_facts(&path);

        // WHEN: The read-back value is written back to the same file and read again
        crate::metadata_writer::update_metadata(
            &path,
            first.taken_at,
            first.latitude,
            first.longitude,
        )
        .expect("failed to rewrite read-back metadata");
        let second = read_media_facts(&path);

        // THEN: Both read-backs are identical (no offset/rounding drift)
        assert_eq!(first, second);
    }

    #[test]
    fn read_media_facts_falls_back_to_the_filename_date() {
        // GIVEN: A file with a date-shaped name and content that is not an image
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("20240215_185056.jpg");
        fs::write(&path, b"not an image").expect("failed to write bytes");

        // WHEN: Reading the facts
        let facts = read_media_facts(&path);

        // THEN: The filename fallback supplies the date (FR-011)
        assert_eq!(
            facts.taken_at,
            Some(Utc.with_ymd_and_hms(2024, 2, 15, 18, 50, 56).unwrap())
        );
    }
}
