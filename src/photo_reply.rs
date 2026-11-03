//! The reply shape for a photo.
//!
//! A [`Photo`] row is a database row: it carries no capture date and no
//! coordinates, because those are facts of the file and the database stores
//! neither. Only [`MediaFactsIndex::enrich`] fills them in. This module makes
//! that step the only way to turn a row into a photo response, so a new route
//! cannot serialize an undated row by forgetting a call.

use serde::{Serialize, Serializer};

use crate::db::Photo;
use crate::media_facts::MediaFactsIndex;

/// A photo carrying the file facts a response must show.
#[derive(Debug)]
pub struct PhotoReply(Photo);

impl PhotoReply {
    /// Enriches the row from the file-facts index and marks it as a reply.
    pub fn from_row(mut photo: Photo, facts: &MediaFactsIndex) -> Self {
        facts.enrich(&mut photo);
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
    use crate::media_facts::test_facts_with_coords;

    #[test]
    fn reply_carries_file_facts_while_the_row_stays_raw() {
        let photo = create_test_photo("a.jpg".to_string(), "a".to_string());
        let facts =
            test_facts_with_coords(&[(&photo.file_path, "2012-03-15T10:00:00Z", 52.52, 13.405)]);

        let reply = PhotoReply::from_row(photo, &facts);
        let value = serde_json::to_value(&reply).unwrap();

        assert_eq!(
            value["taken_at"],
            serde_json::json!("2012-03-15T10:00:00Z"),
            "the reply must carry the file's capture date"
        );
        assert_eq!(value["metadata"]["location"]["latitude"], 52.52);
        assert_eq!(value["metadata"]["location"]["longitude"], 13.405);
    }

    #[test]
    fn a_row_without_facts_stays_undated() {
        let photo = create_test_photo("b.jpg".to_string(), "b".to_string());
        let facts = test_facts_with_coords(&[]);

        let reply = PhotoReply::from_row(photo, &facts);
        let value = serde_json::to_value(&reply).unwrap();

        assert!(value["taken_at"].is_null());
        assert!(value["metadata"]["location"].get("latitude").is_none());
    }
}
