use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub id: Option<i64>,
    pub path: String,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: String,
    pub date_taken: Option<DateTime<Utc>>,
    pub date_modified: DateTime<Utc>,
    pub date_indexed: DateTime<Utc>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub orientation: i32,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub location_name: Option<String>,
    pub hash_md5: Option<String>,
    pub hash_sha256: Option<String>,
    pub thumbnail_path: Option<String>,
    pub has_thumbnail: bool,
}

impl Photo {
    #[allow(dead_code)]
    pub fn new_test_photo(path: &str, filename: &str) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Generate unique hashes based on the path to avoid constraint violations
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        let path_hash = hasher.finish();

        let hash_md5 = format!("{:032x}", path_hash);
        let hash_sha256 = format!("{:064x}", path_hash);

        Photo {
            id: None,
            path: path.to_string(),
            filename: filename.to_string(),
            file_size: 1024,
            mime_type: "image/jpeg".to_string(),
            date_taken: Some(Utc::now()),
            date_modified: Utc::now(),
            date_indexed: Utc::now(),
            width: Some(1920),
            height: Some(1080),
            orientation: 1,
            camera_make: Some("Canon".to_string()),
            camera_model: Some("EOS 5D".to_string()),
            iso: Some(200),
            aperture: Some(2.8),
            shutter_speed: Some("1/60".to_string()),
            focal_length: Some(50.0),
            gps_latitude: Some(37.7749),
            gps_longitude: Some(-122.4194),
            location_name: Some("San Francisco".to_string()),
            hash_md5: Some(hash_md5),
            hash_sha256: Some(hash_sha256),
            thumbnail_path: None,
            has_thumbnail: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_photo_creation() {
        let photo = Photo::new_test_photo("/test/path.jpg", "test.jpg");

        assert_eq!(photo.path, "/test/path.jpg");
        assert_eq!(photo.filename, "test.jpg");
        assert_eq!(photo.file_size, 1024);
        assert_eq!(photo.mime_type, "image/jpeg");
        assert!(photo.date_taken.is_some());
        assert_eq!(photo.width, Some(1920));
        assert_eq!(photo.height, Some(1080));
        assert_eq!(photo.orientation, 1);
        assert_eq!(photo.camera_make, Some("Canon".to_string()));
        assert_eq!(photo.camera_model, Some("EOS 5D".to_string()));
        assert_eq!(photo.iso, Some(200));
        assert_eq!(photo.aperture, Some(2.8));
        assert_eq!(photo.shutter_speed, Some("1/60".to_string()));
        assert_eq!(photo.focal_length, Some(50.0));
        assert_eq!(photo.gps_latitude, Some(37.7749));
        assert_eq!(photo.gps_longitude, Some(-122.4194));
        assert_eq!(photo.location_name, Some("San Francisco".to_string()));
        assert!(photo.hash_md5.is_some());
        assert!(photo.hash_sha256.is_some());
        assert_eq!(photo.has_thumbnail, false);
    }

    #[test]
    fn test_photo_serialization() {
        let photo = Photo::new_test_photo("/test/serialize.jpg", "serialize.jpg");

        let json = serde_json::to_string(&photo).unwrap();
        assert!(json.contains("serialize.jpg"));
        assert!(json.contains("/test/serialize.jpg"));

        let deserialized: Photo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.filename, photo.filename);
        assert_eq!(deserialized.path, photo.path);
    }

    #[test]
    fn test_photo_clone() {
        let photo = Photo::new_test_photo("/test/clone.jpg", "clone.jpg");
        let cloned = photo.clone();

        assert_eq!(photo.filename, cloned.filename);
        assert_eq!(photo.path, cloned.path);
        assert_eq!(photo.file_size, cloned.file_size);
    }
}
