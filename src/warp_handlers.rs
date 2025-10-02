// Re-exports for backward compatibility
pub use crate::handlers_health::{health_check, ready_check};
pub use crate::handlers_photo::{
    get_photo, get_photo_file, get_stats, get_timeline, list_photos, toggle_favorite,
    FavoriteRequest, PhotoQuery,
};
pub use crate::handlers_thumbnail::{get_photo_thumbnail, get_thumbnail_by_hash, ThumbnailQuery};
pub use crate::handlers_video::{get_video_file, VideoQuery};

#[cfg(test)]
mod tests {
    use crate::db::{create_test_db_pool, Photo, SearchQuery};
    use chrono::Utc;

    fn create_test_photo(hash: &str, filename: &str, is_favorite: bool) -> Photo {
        Photo {
            hash_sha256: hash.to_string(),
            file_path: format!("./{}", filename),
            filename: filename.to_string(),
            file_size: 1000,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: Some(Utc::now()),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            camera_make: None,
            camera_model: None,
            lens_make: None,
            lens_model: None,
            iso: None,
            aperture: None,
            shutter_speed: None,
            focal_length: None,
            width: None,
            height: None,
            color_space: None,
            white_balance: None,
            exposure_mode: None,
            metering_mode: None,
            orientation: None,
            flash_used: None,
            latitude: None,
            longitude: None,
            location_name: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            country: None,
            keywords: None,
            faces_detected: None,
            objects_detected: None,
            colors: None,
            duration: None,
            video_codec: None,
            audio_codec: None,
            bitrate: None,
            frame_rate: None,
            is_favorite: Some(is_favorite),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_list_photos_with_favorite_query() {
        // Create test database
        let db_pool = create_test_db_pool().expect("Failed to create test DB");

        // Insert test photos (hash must be 64 chars for SHA256)
        let photo1 = create_test_photo("a".repeat(64).as_str(), "test1.jpg", true);
        let photo2 = create_test_photo("b".repeat(64).as_str(), "test2.jpg", false);

        photo1
            .create_or_update(&db_pool)
            .expect("Failed to insert photo1");
        photo2
            .create_or_update(&db_pool)
            .expect("Failed to insert photo2");

        // Test: Query with is_favorite:true should return only favorited photos
        let search_query = SearchQuery {
            q: Some("is_favorite:true".to_string()),
            camera_make: None,
            camera_model: None,
            year: None,
            month: None,
            keywords: None,
            has_location: None,
            country: None,
            limit: Some(50),
            page: Some(1),
            sort: None,
            order: None,
        };

        let result = Photo::search_photos(&db_pool, &search_query, 50, 0, None, None);
        assert!(result.is_ok());

        let (photos, total) = result.unwrap();
        assert_eq!(total, 1, "Should return only 1 favorite photo");
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].hash_sha256, "a".repeat(64));
        assert_eq!(photos[0].is_favorite, Some(true));
    }

    #[tokio::test]
    async fn test_list_photos_without_query_returns_all() {
        // Create test database
        let db_pool = create_test_db_pool().expect("Failed to create test DB");

        // Insert test photos (hash must be 64 chars for SHA256)
        let photo1 = create_test_photo("c".repeat(64).as_str(), "test3.jpg", true);
        let photo2 = create_test_photo("d".repeat(64).as_str(), "test4.jpg", false);

        photo1
            .create_or_update(&db_pool)
            .expect("Failed to insert photo1");
        photo2
            .create_or_update(&db_pool)
            .expect("Failed to insert photo2");

        // Test: list_with_pagination should return all photos
        let result = Photo::list_with_pagination(&db_pool, 50, 0, None, None);
        assert!(result.is_ok());

        let (photos, total) = result.unwrap();
        assert_eq!(total, 2, "Should return all 2 photos");
        assert_eq!(photos.len(), 2);
    }
}
