//! File-derived capture facts, keyed by photo file path.
//!
//! The photo file is the only place a capture date or coordinate lives; this
//! module is the in-memory index every scan rebuilds and every response is
//! enriched from. The DB stores neither.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use chrono::{DateTime, Utc};

use crate::db::Photo;
use crate::metadata_extractor::MetadataExtractor;

/// Capture facts read from a photo file.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaFacts {
    pub taken_at: Option<DateTime<Utc>>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Process-wide in-memory index of file-derived capture facts, keyed by the
/// photo's file path (a path survives the in-app content-hash re-key on
/// rotation and is what every DB row already carries).
pub struct MediaFactsIndex {
    entries: RwLock<HashMap<String, MediaFacts>>,
}

impl Default for MediaFactsIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaFactsIndex {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, file_path: &str) -> Option<MediaFacts> {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(file_path)
            .copied()
    }

    pub fn set(&self, file_path: &str, facts: MediaFacts) {
        self.entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(file_path.to_string(), facts);
    }

    pub fn remove(&self, file_path: &str) {
        self.entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(file_path);
    }

    pub fn remove_many(&self, file_paths: &[String]) {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for file_path in file_paths {
            entries.remove(file_path);
        }
    }

    /// Reads the file and stores what it yields; returns that value.
    pub fn reload(&self, file_path: &str) -> MediaFacts {
        let facts = read_media_facts(Path::new(file_path));
        self.set(file_path, facts);
        facts
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    }

    /// Attach the file's facts to `photo` for a response: `taken_at` and the
    /// coordinate pair inside `metadata.location` (merging, never dropping an
    /// existing `city`). A photo with no index entry stays undated and keeps
    /// its metadata untouched. Response-only: the DB has no taken_at column
    /// and strips coordinate keys before storing.
    pub fn enrich(&self, photo: &mut Photo) {
        let Some(facts) = self.get(&photo.file_path) else {
            return;
        };

        if facts.taken_at.is_some() {
            photo.taken_at = facts.taken_at;
        }

        let (Some(latitude), Some(longitude)) = (facts.latitude, facts.longitude) else {
            return;
        };

        let mut location = photo
            .metadata
            .get("location")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        location.insert("latitude".to_string(), serde_json::json!(latitude));
        location.insert("longitude".to_string(), serde_json::json!(longitude));

        if !photo.metadata.is_object() {
            photo.metadata = serde_json::Value::Object(serde_json::Map::new());
        }
        if let Some(metadata) = photo.metadata.as_object_mut() {
            metadata.insert("location".to_string(), serde_json::Value::Object(location));
        }
    }
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

/// Test-only index filled from `(path, RFC3339 date)` entries.
#[cfg(test)]
pub(crate) fn test_facts(entries: &[(&str, &str)]) -> MediaFactsIndex {
    let index = MediaFactsIndex::new();
    for (path, taken_at) in entries {
        index.set(
            path,
            MediaFacts {
                taken_at: Some(
                    DateTime::parse_from_rfc3339(taken_at)
                        .expect("invalid RFC3339 date in test facts")
                        .with_timezone(&Utc),
                ),
                ..MediaFacts::default()
            },
        );
    }
    index
}

/// Test-only index filled from `(path, RFC3339 date, latitude, longitude)`
/// entries.
#[cfg(test)]
pub(crate) fn test_facts_with_coords(entries: &[(&str, &str, f64, f64)]) -> MediaFactsIndex {
    let index = MediaFactsIndex::new();
    for (path, taken_at, latitude, longitude) in entries {
        index.set(
            path,
            MediaFacts {
                taken_at: Some(
                    DateTime::parse_from_rfc3339(taken_at)
                        .expect("invalid RFC3339 date in test facts")
                        .with_timezone(&Utc),
                ),
                latitude: Some(*latitude),
                longitude: Some(*longitude),
            },
        );
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::TimeZone;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    use crate::db::Photo;

    fn copy_fixture(temp_dir: &TempDir) -> PathBuf {
        let source = Path::new("test-data/IMG_9377.jpg");
        assert!(source.exists(), "test fixture missing: {source:?}");
        let dest = temp_dir.path().join("photo.jpg");
        fs::copy(source, &dest).expect("failed to copy test fixture");
        dest
    }

    fn test_photo(file_path: &str, metadata: serde_json::Value) -> Photo {
        let now = Utc::now();
        Photo {
            hash_sha256: "a".repeat(64),
            file_path: file_path.to_string(),
            filename: "photo.jpg".to_string(),
            file_size: 1024,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            width: None,
            height: None,
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: None,
            blurhash: None,
            is_favorite: None,
            semantic_vector_indexed: None,
            metadata,
            date_modified: now,
            date_indexed: None,
            created_at: now,
            updated_at: now,
        }
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

    #[test]
    fn index_reload_reads_the_file_and_stores_what_it_yields() {
        // GIVEN: A real JPEG fixture with a written date and coordinates
        let temp_dir = TempDir::new().unwrap();
        let path = copy_fixture(&temp_dir);
        let path = path.to_string_lossy().to_string();
        let written = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 0).unwrap();
        crate::metadata_writer::update_metadata(
            Path::new(&path),
            Some(written),
            Some(40.7128),
            Some(-74.0060),
        )
        .expect("failed to write metadata");
        let index = MediaFactsIndex::new();

        // WHEN: Reloading that path
        let reloaded = index.reload(&path);

        // THEN: The returned facts are the file's facts...
        assert_eq!(reloaded.taken_at, Some(written));
        // AND: ...and they are published to the index
        assert_eq!(index.get(&path), Some(reloaded));
    }

    #[test]
    fn enrich_sets_the_date_and_merges_coordinates() {
        // GIVEN: An index entry with a date and coordinates for the photo's path
        let index =
            test_facts_with_coords(&[("/tmp/a.jpg", "2012-03-15T10:00:00Z", 52.52, 13.405)]);
        let mut photo = test_photo(
            "/tmp/a.jpg",
            serde_json::json!({"location": {"city": "Berlin"}}),
        );

        // WHEN: Enriching the photo
        index.enrich(&mut photo);

        // THEN: The date comes from the file
        assert_eq!(
            photo.taken_at,
            Some(Utc.with_ymd_and_hms(2012, 3, 15, 10, 0, 0).unwrap())
        );
        // AND: The coordinates merge into location without dropping the city
        let location = &photo.metadata["location"];
        assert_eq!(location["city"], serde_json::json!("Berlin"));
        assert!((location["latitude"].as_f64().unwrap() - 52.52).abs() < 1e-9);
        assert!((location["longitude"].as_f64().unwrap() - 13.405).abs() < 1e-9);
    }

    #[test]
    fn enrich_leaves_unknown_photos_undated() {
        // GIVEN: An empty index and a photo whose metadata has only a city
        let index = test_facts(&[]);
        let metadata = serde_json::json!({"location": {"city": "Berlin"}});
        let mut photo = test_photo("/tmp/missing.jpg", metadata.clone());

        // WHEN: Enriching the photo
        index.enrich(&mut photo);

        // THEN: The photo stays undated and its metadata is untouched
        assert!(photo.taken_at.is_none());
        assert_eq!(photo.metadata, metadata);
    }

    #[test]
    fn index_set_remove_and_len() {
        // GIVEN: An index with two entries
        let index = test_facts(&[
            ("/tmp/a.jpg", "2012-03-15T10:00:00Z"),
            ("/tmp/b.jpg", "2013-01-01T00:00:00Z"),
        ]);
        assert_eq!(index.len(), 2);
        assert!(!index.is_empty());

        // WHEN: Removing one path
        index.remove("/tmp/a.jpg");

        // THEN: Only that entry is gone
        assert_eq!(index.len(), 1);
        assert!(index.get("/tmp/a.jpg").is_none());
        assert!(index.get("/tmp/b.jpg").is_some());

        // WHEN: Removing the rest in bulk
        index.remove_many(&["/tmp/b.jpg".to_string()]);

        // THEN: The index is empty
        assert_eq!(index.len(), 0);
        assert!(index.is_empty());
    }
}
