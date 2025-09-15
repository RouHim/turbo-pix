use crate::db::{DbPool, Photo};
use chrono::{DateTime, Utc};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub min_width: Option<i32>,
    pub min_height: Option<i32>,
    pub has_gps: Option<bool>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchSuggestion {
    pub field: String,
    pub value: String,
    pub count: usize,
}

pub struct PhotoCrud;

impl PhotoCrud {
    pub fn find_by_id(
        conn: &mut r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>,
        id: i32,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let mut stmt = conn.prepare(
            "SELECT id, path, filename, file_size, mime_type, date_taken, date_modified, date_indexed,
             width, height, orientation, camera_make, camera_model, iso, aperture, shutter_speed,
             focal_length, gps_latitude, gps_longitude, location_name, hash_md5, hash_sha256, thumbnail_path, has_thumbnail
             FROM photos WHERE id = ?1"
        )?;

        let mut photo_iter = stmt.query_map(params![id as i64], Photo::from_row)?;

        if let Some(photo) = photo_iter.next() {
            Ok(Some(photo?))
        } else {
            Ok(None)
        }
    }
}

impl Photo {
    pub fn from_row(row: &Row) -> Result<Photo, rusqlite::Error> {
        Ok(Photo {
            id: Some(row.get(0)?),
            path: row.get(1)?,
            filename: row.get(2)?,
            file_size: row.get(3)?,
            mime_type: row.get(4)?,
            date_taken: row
                .get::<_, Option<String>>(5)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            date_modified: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                .map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        6,
                        "date_modified".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?
                .with_timezone(&Utc),
            date_indexed: DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                .map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        7,
                        "date_indexed".to_string(),
                        rusqlite::types::Type::Text,
                    )
                })?
                .with_timezone(&Utc),
            width: row.get(8)?,
            height: row.get(9)?,
            orientation: row.get(10)?,
            camera_make: row.get(11)?,
            camera_model: row.get(12)?,
            iso: row.get(13)?,
            aperture: row.get(14)?,
            shutter_speed: row.get(15)?,
            focal_length: row.get(16)?,
            gps_latitude: row.get(17)?,
            gps_longitude: row.get(18)?,
            location_name: row.get(19)?,
            hash_md5: row.get(20)?,
            hash_sha256: row.get(21)?,
            thumbnail_path: row.get(22)?,
            has_thumbnail: row.get(23)?,
        })
    }

    pub fn create(&self, pool: &DbPool) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute(
            "INSERT INTO photos (path, filename, file_size, mime_type, date_taken, date_modified, 
             date_indexed, width, height, orientation, camera_make, camera_model, iso, aperture, 
             shutter_speed, focal_length, gps_latitude, gps_longitude, location_name, hash_md5, 
             hash_sha256, thumbnail_path, has_thumbnail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 
             ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                self.path, self.filename, self.file_size, self.mime_type,
                self.date_taken.map(|dt| dt.to_rfc3339()),
                self.date_modified.to_rfc3339(),
                self.date_indexed.to_rfc3339(),
                self.width, self.height, self.orientation, self.camera_make, self.camera_model,
                self.iso, self.aperture, self.shutter_speed, self.focal_length,
                self.gps_latitude, self.gps_longitude, self.location_name, self.hash_md5,
                self.hash_sha256, self.thumbnail_path, self.has_thumbnail
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_all(pool: &DbPool, limit: i64) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, path, filename, file_size, mime_type, date_taken, date_modified, date_indexed,
             width, height, orientation, camera_make, camera_model, iso, aperture, shutter_speed,
             focal_length, gps_latitude, gps_longitude, location_name, hash_md5, hash_sha256, thumbnail_path, has_thumbnail
             FROM photos ORDER BY date_indexed DESC LIMIT ?1"
        )?;

        let photo_iter = stmt.query_map(params![limit], Photo::from_row)?;

        let mut photos = Vec::new();
        for photo in photo_iter {
            photos.push(photo?);
        }
        Ok(photos)
    }

    pub fn list_with_pagination(
        pool: &DbPool,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<Photo>, usize), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Get total count
        let total: usize = conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?;

        // Get paginated results
        let mut stmt = conn.prepare(
            "SELECT id, path, filename, file_size, mime_type, date_taken, date_modified, date_indexed,
             width, height, orientation, camera_make, camera_model, iso, aperture, shutter_speed,
             focal_length, gps_latitude, gps_longitude, location_name, hash_md5, hash_sha256, thumbnail_path, has_thumbnail
             FROM photos ORDER BY date_indexed DESC LIMIT ?1 OFFSET ?2"
        )?;

        let photo_iter = stmt.query_map(params![limit, offset], Photo::from_row)?;

        let mut photos = Vec::new();
        for photo in photo_iter {
            photos.push(photo?);
        }
        Ok((photos, total))
    }

    pub fn find_by_id(pool: &DbPool, id: i64) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, path, filename, file_size, mime_type, date_taken, date_modified, date_indexed,
             width, height, orientation, camera_make, camera_model, iso, aperture, shutter_speed,
             focal_length, gps_latitude, gps_longitude, location_name, hash_md5, hash_sha256, thumbnail_path, has_thumbnail
             FROM photos WHERE id = ?1"
        )?;

        let mut photo_iter = stmt.query_map(params![id], Photo::from_row)?;

        if let Some(photo) = photo_iter.next() {
            Ok(Some(photo?))
        } else {
            Ok(None)
        }
    }

    pub fn find_by_path(
        pool: &DbPool,
        path: &str,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, path, filename, file_size, mime_type, date_taken, date_modified, date_indexed,
             width, height, orientation, camera_make, camera_model, iso, aperture, shutter_speed,
             focal_length, gps_latitude, gps_longitude, location_name, hash_md5, hash_sha256, thumbnail_path, has_thumbnail
             FROM photos WHERE path = ?1"
        )?;

        let mut photo_iter = stmt.query_map(params![path], Photo::from_row)?;

        if let Some(photo) = photo_iter.next() {
            Ok(Some(photo?))
        } else {
            Ok(None)
        }
    }

    #[allow(dead_code)]
    pub fn find_by_hash(
        pool: &DbPool,
        hash: &str,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, path, filename, file_size, mime_type, date_taken, date_modified, date_indexed,
             width, height, orientation, camera_make, camera_model, iso, aperture, shutter_speed,
             focal_length, gps_latitude, gps_longitude, location_name, hash_md5, hash_sha256, thumbnail_path, has_thumbnail
             FROM photos WHERE hash_sha256 = ?1"
        )?;

        let mut photo_iter = stmt.query_map(params![hash], Photo::from_row)?;

        if let Some(photo) = photo_iter.next() {
            Ok(Some(photo?))
        } else {
            Ok(None)
        }
    }

    pub fn create_or_update(&self, pool: &DbPool) -> Result<i64, Box<dyn std::error::Error>> {
        if let Some(existing) = Self::find_by_path(pool, &self.path)? {
            self.update(pool, existing.id.unwrap())
        } else {
            self.create(pool)
        }
    }

    pub fn update(&self, pool: &DbPool, id: i64) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE photos SET filename = ?2, file_size = ?3, mime_type = ?4, date_taken = ?5, 
             date_modified = ?6, date_indexed = ?7, width = ?8, height = ?9, orientation = ?10,
             camera_make = ?11, camera_model = ?12, iso = ?13, aperture = ?14, shutter_speed = ?15,
             focal_length = ?16, gps_latitude = ?17, gps_longitude = ?18, location_name = ?19,
             hash_md5 = ?20, hash_sha256 = ?21, thumbnail_path = ?22, has_thumbnail = ?23
             WHERE id = ?1",
            params![
                id,
                self.filename,
                self.file_size,
                self.mime_type,
                self.date_taken.map(|dt| dt.to_rfc3339()),
                self.date_modified.to_rfc3339(),
                self.date_indexed.to_rfc3339(),
                self.width,
                self.height,
                self.orientation,
                self.camera_make,
                self.camera_model,
                self.iso,
                self.aperture,
                self.shutter_speed,
                self.focal_length,
                self.gps_latitude,
                self.gps_longitude,
                self.location_name,
                self.hash_md5,
                self.hash_sha256,
                self.thumbnail_path,
                self.has_thumbnail
            ],
        )?;
        Ok(id)
    }

    pub fn delete(pool: &DbPool, id: i64) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let rows_affected = conn.execute("DELETE FROM photos WHERE id = ?1", params![id])?;
        Ok(rows_affected > 0)
    }

    pub fn update_thumbnail_status(
        &self,
        pool: &DbPool,
        has_thumbnail: bool,
        thumbnail_path: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(id) = self.id {
            let conn = pool.get()?;
            conn.execute(
                "UPDATE photos SET has_thumbnail = ?1, thumbnail_path = ?2 WHERE id = ?3",
                params![has_thumbnail, thumbnail_path, id],
            )?;
        }
        Ok(())
    }

    pub fn search_photos(
        pool: &DbPool,
        query: &SearchQuery,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<Photo>, usize), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        // Build WHERE clause based on search parameters
        if let Some(q) = &query.q {
            where_clauses.push("(filename LIKE ?1 OR camera_make LIKE ?1 OR camera_model LIKE ?1 OR location_name LIKE ?1)");
            params_vec.push(Box::new(format!("%{}%", q)));
        }

        if let Some(date_from) = &query.date_from {
            where_clauses.push("date_taken >= ?");
            params_vec.push(Box::new(date_from.clone()));
        }

        if let Some(date_to) = &query.date_to {
            where_clauses.push("date_taken <= ?");
            params_vec.push(Box::new(date_to.clone()));
        }

        if let Some(camera_make) = &query.camera_make {
            where_clauses.push("camera_make = ?");
            params_vec.push(Box::new(camera_make.clone()));
        }

        if let Some(camera_model) = &query.camera_model {
            where_clauses.push("camera_model = ?");
            params_vec.push(Box::new(camera_model.clone()));
        }

        if let Some(min_width) = query.min_width {
            where_clauses.push("width >= ?");
            params_vec.push(Box::new(min_width));
        }

        if let Some(min_height) = query.min_height {
            where_clauses.push("height >= ?");
            params_vec.push(Box::new(min_height));
        }

        if let Some(has_gps) = query.has_gps {
            if has_gps {
                where_clauses.push("gps_latitude IS NOT NULL AND gps_longitude IS NOT NULL");
            } else {
                where_clauses.push("gps_latitude IS NULL OR gps_longitude IS NULL");
            }
        }

        if let Some(mime_type) = &query.mime_type {
            where_clauses.push("mime_type = ?");
            params_vec.push(Box::new(mime_type.clone()));
        }

        let where_clause = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let sort_field = query.sort.as_deref().unwrap_or("date_indexed");
        let sort_order = query.order.as_deref().unwrap_or("desc");

        // Validate sort field to prevent SQL injection
        let valid_sort_field = match sort_field {
            "date_taken" | "date_indexed" | "filename" | "file_size" | "camera_make"
            | "camera_model" => sort_field,
            _ => "date_indexed",
        };

        let valid_sort_order = match sort_order.to_lowercase().as_str() {
            "asc" => "ASC",
            "desc" => "DESC",
            _ => "DESC",
        };

        // Get total count
        let count_sql = format!("SELECT COUNT(*) FROM photos {}", where_clause);
        let total: usize = if params_vec.is_empty() {
            conn.query_row(&count_sql, [], |row| row.get(0))?
        } else {
            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            conn.query_row(&count_sql, &params_refs[..], |row| row.get(0))?
        };

        // Get paginated results
        let search_sql = format!(
            "SELECT id, path, filename, file_size, mime_type, date_taken, date_modified, date_indexed,
             width, height, orientation, camera_make, camera_model, iso, aperture, shutter_speed,
             focal_length, gps_latitude, gps_longitude, location_name, hash_md5, hash_sha256, thumbnail_path, has_thumbnail
             FROM photos {} ORDER BY {} {} LIMIT ? OFFSET ?",
            where_clause, valid_sort_field, valid_sort_order
        );

        // Add limit and offset to params
        params_vec.push(Box::new(limit));
        params_vec.push(Box::new(offset));

        let mut stmt = conn.prepare(&search_sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let photo_iter = stmt.query_map(&params_refs[..], Photo::from_row)?;

        let mut photos = Vec::new();
        for photo in photo_iter {
            photos.push(photo?);
        }

        Ok((photos, total))
    }

    pub fn get_search_suggestions(
        pool: &DbPool,
        _query_text: Option<&str>,
    ) -> Result<Vec<SearchSuggestion>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut suggestions = Vec::new();

        // Camera makes
        let mut stmt = conn.prepare("SELECT camera_make, COUNT(*) FROM photos WHERE camera_make IS NOT NULL GROUP BY camera_make ORDER BY COUNT(*) DESC LIMIT 10")?;
        let camera_makes = stmt.query_map([], |row| {
            Ok(SearchSuggestion {
                field: "camera_make".to_string(),
                value: row.get::<_, String>(0)?,
                count: row.get::<_, usize>(1)?,
            })
        })?;

        for suggestion in camera_makes {
            suggestions.push(suggestion?);
        }

        // Camera models
        let mut stmt = conn.prepare("SELECT camera_model, COUNT(*) FROM photos WHERE camera_model IS NOT NULL GROUP BY camera_model ORDER BY COUNT(*) DESC LIMIT 10")?;
        let camera_models = stmt.query_map([], |row| {
            Ok(SearchSuggestion {
                field: "camera_model".to_string(),
                value: row.get::<_, String>(0)?,
                count: row.get::<_, usize>(1)?,
            })
        })?;

        for suggestion in camera_models {
            suggestions.push(suggestion?);
        }

        // Locations
        let mut stmt = conn.prepare("SELECT location_name, COUNT(*) FROM photos WHERE location_name IS NOT NULL GROUP BY location_name ORDER BY COUNT(*) DESC LIMIT 10")?;
        let locations = stmt.query_map([], |row| {
            Ok(SearchSuggestion {
                field: "location".to_string(),
                value: row.get::<_, String>(0)?,
                count: row.get::<_, usize>(1)?,
            })
        })?;

        for suggestion in locations {
            suggestions.push(suggestion?);
        }

        Ok(suggestions)
    }

    pub fn get_cameras(
        pool: &DbPool,
    ) -> Result<Vec<crate::web::handlers::photos::Camera>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut cameras = Vec::new();

        let mut stmt = conn.prepare(
            "SELECT camera_make, camera_model, COUNT(*) 
             FROM photos 
             WHERE camera_make IS NOT NULL AND camera_model IS NOT NULL 
             GROUP BY camera_make, camera_model 
             ORDER BY COUNT(*) DESC",
        )?;

        let camera_iter = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, usize>(2)?,
            ))
        })?;

        for camera in camera_iter {
            let (make, model, count) = camera?;
            cameras.push(crate::web::handlers::photos::Camera {
                make,
                model,
                photo_count: count,
            });
        }

        Ok(cameras)
    }

    pub fn get_stats(
        pool: &DbPool,
    ) -> Result<crate::web::handlers::photos::StatsResponse, Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        let total_photos: usize =
            conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?;
        let total_size: i64 = conn.query_row(
            "SELECT COALESCE(SUM(file_size), 0) FROM photos",
            [],
            |row| row.get(0),
        )?;

        let cameras = Self::get_cameras(pool)?;

        Ok(crate::web::handlers::photos::StatsResponse {
            total_photos,
            total_size,
            cameras,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::create_in_memory_pool;
    use tempfile::TempDir;

    fn setup_test_db() -> DbPool {
        create_in_memory_pool().unwrap()
    }

    #[test]
    fn test_photo_create_and_find_by_path() {
        let pool = setup_test_db();
        let photo = Photo::new_test_photo("/test/create.jpg", "create.jpg");

        let id = photo.create(&pool).unwrap();
        assert!(id > 0);

        let found = Photo::find_by_path(&pool, "/test/create.jpg").unwrap();
        assert!(found.is_some());

        let found_photo = found.unwrap();
        assert_eq!(found_photo.path, "/test/create.jpg");
        assert_eq!(found_photo.filename, "create.jpg");
        assert_eq!(found_photo.id, Some(id));
    }

    #[test]
    fn test_photo_find_by_hash() {
        let pool = setup_test_db();
        let photo = Photo::new_test_photo("/test/hash.jpg", "hash.jpg");

        photo.create(&pool).unwrap();

        let found = Photo::find_by_hash(&pool, &photo.hash_sha256.clone().unwrap()).unwrap();
        assert!(found.is_some());

        let found_photo = found.unwrap();
        assert_eq!(found_photo.path, "/test/hash.jpg");
        assert_eq!(found_photo.hash_sha256, photo.hash_sha256);
    }

    #[test]
    fn test_photo_list_all() {
        let pool = setup_test_db();

        let photo1 = Photo::new_test_photo("/test/list1.jpg", "list1.jpg");
        let photo2 = Photo::new_test_photo("/test/list2.jpg", "list2.jpg");

        photo1.create(&pool).unwrap();
        photo2.create(&pool).unwrap();

        let photos = Photo::list_all(&pool, 10).unwrap();
        assert_eq!(photos.len(), 2);

        assert!(photos.iter().any(|p| p.filename == "list1.jpg"));
        assert!(photos.iter().any(|p| p.filename == "list2.jpg"));
    }

    #[test]
    fn test_photo_update() {
        let pool = setup_test_db();
        let mut photo = Photo::new_test_photo("/test/update.jpg", "update.jpg");

        let id = photo.create(&pool).unwrap();

        photo.filename = "updated.jpg".to_string();
        photo.file_size = 2048;
        photo.camera_make = Some("Nikon".to_string());

        let updated_id = photo.update(&pool, id).unwrap();
        assert_eq!(updated_id, id);

        let found = Photo::find_by_path(&pool, "/test/update.jpg")
            .unwrap()
            .unwrap();
        assert_eq!(found.filename, "updated.jpg");
        assert_eq!(found.file_size, 2048);
        assert_eq!(found.camera_make, Some("Nikon".to_string()));
    }

    #[test]
    fn test_photo_create_or_update_new() {
        let pool = setup_test_db();
        let photo = Photo::new_test_photo("/test/create_or_update_new.jpg", "new.jpg");

        let id = photo.create_or_update(&pool).unwrap();
        assert!(id > 0);

        let found = Photo::find_by_path(&pool, "/test/create_or_update_new.jpg").unwrap();
        assert!(found.is_some());
    }

    #[test]
    fn test_photo_create_or_update_existing() {
        let pool = setup_test_db();
        let mut photo =
            Photo::new_test_photo("/test/create_or_update_existing.jpg", "existing.jpg");

        let original_id = photo.create(&pool).unwrap();

        photo.filename = "updated_existing.jpg".to_string();
        photo.file_size = 4096;

        let updated_id = photo.create_or_update(&pool).unwrap();
        assert_eq!(updated_id, original_id);

        let found = Photo::find_by_path(&pool, "/test/create_or_update_existing.jpg")
            .unwrap()
            .unwrap();
        assert_eq!(found.filename, "updated_existing.jpg");
        assert_eq!(found.file_size, 4096);
        assert_eq!(found.id, Some(original_id));
    }

    #[test]
    fn test_photo_find_nonexistent() {
        let pool = setup_test_db();

        let found = Photo::find_by_path(&pool, "/nonexistent/path.jpg").unwrap();
        assert!(found.is_none());

        let found_by_hash = Photo::find_by_hash(&pool, "nonexistent_hash").unwrap();
        assert!(found_by_hash.is_none());
    }

    #[test]
    fn test_photo_list_all_with_limit() {
        let pool = setup_test_db();

        for i in 0..5 {
            let photo =
                Photo::new_test_photo(&format!("/test/limit{}.jpg", i), &format!("limit{}.jpg", i));
            photo.create(&pool).unwrap();
        }

        let photos = Photo::list_all(&pool, 3).unwrap();
        assert_eq!(photos.len(), 3);
    }

    #[test]
    fn test_photo_delete() {
        let pool = setup_test_db();
        let photo = Photo::new_test_photo("/test/delete.jpg", "delete.jpg");

        let id = photo.create(&pool).unwrap();

        // Verify photo exists
        let found = Photo::find_by_id(&pool, id).unwrap();
        assert!(found.is_some());

        // Delete photo
        let deleted = Photo::delete(&pool, id).unwrap();
        assert!(deleted);

        // Verify photo is gone
        let found_after = Photo::find_by_id(&pool, id).unwrap();
        assert!(found_after.is_none());
    }

    #[test]
    fn test_photo_delete_nonexistent() {
        let pool = setup_test_db();

        let deleted = Photo::delete(&pool, 99999).unwrap();
        assert!(!deleted);
    }
}
