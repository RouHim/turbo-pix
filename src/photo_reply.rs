//! The reply shape for a photo.
//!
//! A [`Photo`] row carries its derived coordinates as flat columns
//! (`taken_latitude` / `taken_longitude`, skipped in serialization), while the
//! API nests them under `metadata.location`. This module is that one
//! formatting step, so no route can serialize a photo whose coordinates are
//! missing from the shape the client reads.

use serde::{Serialize, Serializer};

use crate::db::Photo;

/// A photo carrying the file facts in the API's shape.
#[derive(Debug)]
pub struct PhotoReply(Photo);

impl PhotoReply {
    /// Nests the row's derived coordinate pair into `metadata.location`.
    pub fn from_row(mut photo: Photo) -> Self {
        let (Some(latitude), Some(longitude)) = (photo.taken_latitude, photo.taken_longitude)
        else {
            return Self(photo);
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
        Self(photo)
    }

    /// The enriched row, for a route that projects a custom payload instead of
    /// serializing the photo itself.
    pub fn into_inner(self) -> Photo {
        self.0
    }
}

impl Serialize for PhotoReply {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::create_test_photo;

    #[test]
    fn reply_nests_the_row_coordinates_under_location() {
        let mut photo = create_test_photo("a.jpg".to_string(), "a".to_string());
        photo.taken_latitude = Some(52.52);
        photo.taken_longitude = Some(13.405);
        photo.metadata = serde_json::json!({"location": {"city": "Berlin"}});

        let reply = PhotoReply::from_row(photo);
        let value = serde_json::to_value(&reply).unwrap();

        assert_eq!(value["metadata"]["location"]["city"], "Berlin");
        assert_eq!(value["metadata"]["location"]["latitude"], 52.52);
        assert_eq!(value["metadata"]["location"]["longitude"], 13.405);
        // The flat columns are an implementation detail, never in the payload.
        assert!(value.get("taken_latitude").is_none());
        assert!(value.get("taken_longitude").is_none());
    }

    #[test]
    fn a_row_without_coordinates_keeps_its_metadata_untouched() {
        let mut photo = create_test_photo("b.jpg".to_string(), "b".to_string());
        photo.metadata = serde_json::json!({"location": {"city": "Berlin"}});
        let metadata = photo.metadata.clone();

        let reply = PhotoReply::from_row(photo);
        let value = serde_json::to_value(&reply).unwrap();

        assert_eq!(value["metadata"], metadata);
        assert!(value["taken_at"].is_null());
    }
}
