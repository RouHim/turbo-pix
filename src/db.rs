use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{params, Result as SqlResult, Row};
use serde::{Deserialize, Serialize};

pub use crate::db_pool::{
    create_db_pool, delete_orphaned_photos, get_all_photo_paths, vacuum_database, DbPool,
};
pub use crate::db_types::{SearchQuery, SearchSuggestion, TimelineData, TimelineDensity};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub hash_sha256: String,
    pub file_path: String,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: Option<String>,
    pub taken_at: Option<DateTime<Utc>>,
    pub date_modified: DateTime<Utc>,
    pub date_indexed: Option<DateTime<Utc>>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub focal_length: Option<f64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub color_space: Option<String>,
    pub white_balance: Option<String>,
    pub exposure_mode: Option<String>,
    pub metering_mode: Option<String>,
    pub orientation: Option<i32>,
    pub flash_used: Option<bool>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub location_name: Option<String>,
    pub thumbnail_path: Option<String>,
    pub has_thumbnail: Option<bool>,
    pub country: Option<String>,
    pub keywords: Option<String>,
    pub faces_detected: Option<String>,
    pub objects_detected: Option<String>,
    pub colors: Option<String>,
    pub duration: Option<f64>,       // Video duration in seconds
    pub video_codec: Option<String>, // Video codec (e.g., "h264", "h265")
    pub audio_codec: Option<String>, // Audio codec (e.g., "aac", "mp3")
    pub bitrate: Option<i32>,        // Bitrate in kbps
    pub frame_rate: Option<f64>,     // Frame rate for videos
    pub is_favorite: Option<bool>,   // Whether photo is marked as favorite
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Photo {
    pub fn from_row(row: &Row) -> SqlResult<Self> {
        Ok(Photo {
            hash_sha256: row.get(0)?, // Now first column (PRIMARY KEY)
            file_path: row.get(1)?,
            filename: row.get(2)?,
            file_size: row.get(3)?,
            mime_type: row.get(4)?,
            taken_at: row.get::<_, Option<String>>(5)?.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            date_modified: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                .unwrap()
                .with_timezone(&Utc),
            date_indexed: row.get::<_, Option<String>>(7)?.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            camera_make: row.get(8)?,
            camera_model: row.get(9)?,
            lens_make: row.get(10)?,
            lens_model: row.get(11)?,
            iso: row.get(12)?,
            aperture: row.get(13)?,
            shutter_speed: row.get(14)?,
            focal_length: row.get(15)?,
            width: row.get(16)?,
            height: row.get(17)?,
            color_space: row.get(18)?,
            white_balance: row.get(19)?,
            exposure_mode: row.get(20)?,
            metering_mode: row.get(21)?,
            orientation: row.get(22)?,
            flash_used: row.get(23)?,
            latitude: row.get(24)?,
            longitude: row.get(25)?,
            location_name: row.get(26)?,
            thumbnail_path: row.get(27)?, // hash_sha256 removed from index 27
            has_thumbnail: row.get(28)?,
            country: row.get(29)?,
            keywords: row.get(30)?,
            faces_detected: row.get(31)?,
            objects_detected: row.get(32)?,
            colors: row.get(33)?,
            duration: row.get(34)?,
            video_codec: row.get(35)?,
            audio_codec: row.get(36)?,
            bitrate: row.get(37)?,
            frame_rate: row.get(38)?,
            is_favorite: row.get(39)?,
            created_at: {
                let datetime_str = row.get::<_, String>(40)?;
                if datetime_str.contains('T') {
                    DateTime::parse_from_rfc3339(&datetime_str)
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                40,
                                "created_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .with_timezone(&Utc)
                } else {
                    NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                40,
                                "created_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .and_utc()
                }
            },
            updated_at: {
                let datetime_str = row.get::<_, String>(41)?;
                if datetime_str.contains('T') {
                    DateTime::parse_from_rfc3339(&datetime_str)
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                41,
                                "updated_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .with_timezone(&Utc)
                } else {
                    NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                41,
                                "updated_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .and_utc()
                }
            },
        })
    }

    pub fn update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE photos SET
                 file_path = ?, filename = ?, file_size = ?, mime_type = ?,
                 taken_at = ?, file_modified = ?, date_indexed = ?,
                 camera_make = ?, camera_model = ?, lens_make = ?, lens_model = ?,
                 iso = ?, aperture = ?, shutter_speed = ?, focal_length = ?,
                 width = ?, height = ?, color_space = ?, white_balance = ?,
                 exposure_mode = ?, metering_mode = ?, orientation = ?, flash_used = ?,
                 latitude = ?, longitude = ?, location_name = ?,
                 thumbnail_path = ?, has_thumbnail = ?,
                 country = ?, keywords = ?, faces_detected = ?, objects_detected = ?, colors = ?,
                 duration = ?, video_codec = ?, audio_codec = ?, bitrate = ?, frame_rate = ?,
                 is_favorite = ?, updated_at = ?
              WHERE hash_sha256 = ?",
            rusqlite::params![
                self.file_path,
                self.filename,
                self.file_size,
                self.mime_type,
                self.taken_at.map(|dt| dt.to_rfc3339()),
                self.date_modified.to_rfc3339(),
                self.date_indexed.map(|dt| dt.to_rfc3339()),
                self.camera_make,
                self.camera_model,
                self.lens_make,
                self.lens_model,
                self.iso,
                self.aperture,
                self.shutter_speed,
                self.focal_length,
                self.width,
                self.height,
                self.color_space,
                self.white_balance,
                self.exposure_mode,
                self.metering_mode,
                self.orientation,
                self.flash_used,
                self.latitude,
                self.longitude,
                self.location_name,
                self.thumbnail_path,
                self.has_thumbnail,
                self.country,
                self.keywords,
                self.faces_detected,
                self.objects_detected,
                self.colors,
                self.duration,
                self.video_codec,
                self.audio_codec,
                self.bitrate,
                self.frame_rate,
                self.is_favorite.unwrap_or(false),
                Utc::now().to_rfc3339(),
                self.hash_sha256
            ],
        )?;
        Ok(())
    }

    pub fn create_or_update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        let existing = conn.query_row(
            "SELECT hash_sha256 FROM photos WHERE hash_sha256 = ?",
            [&self.hash_sha256],
            |row| row.get::<_, String>(0),
        );

        if existing.is_ok() {
            self.update(pool)
        } else {
            self.create(pool)?;
            Ok(())
        }
    }

    pub fn list_with_pagination(
        pool: &DbPool,
        limit: i64,
        offset: i64,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Get total count
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?;

        // Build ORDER BY clause
        let sort_field = match sort {
            Some("filename") | Some("name") => "filename",
            Some("camera_make") => "camera_make",
            Some("camera_model") => "camera_model",
            Some("file_size") | Some("size") => "file_size",
            Some("created_at") => "created_at",
            Some("date") => "taken_at",
            _ => "taken_at", // default
        };

        let sort_order = match order {
            Some("asc") => "ASC",
            _ => "DESC", // default
        };

        // Get paginated results
        let query = format!(
            "SELECT * FROM photos ORDER BY {} {} LIMIT ? OFFSET ?",
            sort_field, sort_order
        );

        let mut stmt = conn.prepare(&query)?;
        let photo_iter = stmt.query_map([limit, offset], Photo::from_row)?;

        let mut photos = Vec::new();
        for photo in photo_iter {
            photos.push(photo?);
        }
        Ok((photos, total))
    }

    pub fn find_by_hash(
        pool: &DbPool,
        hash: &str,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare("SELECT * FROM photos WHERE hash_sha256 = ?")?;

        match stmt.query_row([hash], Photo::from_row) {
            Ok(photo) => Ok(Some(photo)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    pub fn create(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        let sql = r#"
            INSERT INTO photos (
                hash_sha256, file_path, filename, file_size, mime_type, taken_at, file_modified,
                date_indexed, camera_make, camera_model, lens_make, lens_model,
                iso, aperture, shutter_speed, focal_length, width, height, color_space,
                white_balance, exposure_mode, metering_mode, orientation, flash_used,
                latitude, longitude, location_name, thumbnail_path, has_thumbnail,
                country, keywords, faces_detected, objects_detected, colors,
                duration, video_codec, audio_codec, bitrate, frame_rate,
                is_favorite, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42
            )
        "#;

        conn.execute(
            sql,
            params![
                self.hash_sha256,
                self.file_path,
                self.filename,
                self.file_size,
                self.mime_type,
                self.taken_at.map(|dt| dt.to_rfc3339()),
                self.date_modified.to_rfc3339(),
                self.date_indexed.map(|dt| dt.to_rfc3339()),
                self.camera_make,
                self.camera_model,
                self.lens_make,
                self.lens_model,
                self.iso,
                self.aperture,
                self.shutter_speed,
                self.focal_length,
                self.width,
                self.height,
                self.color_space,
                self.white_balance,
                self.exposure_mode,
                self.metering_mode,
                self.orientation,
                self.flash_used,
                self.latitude,
                self.longitude,
                self.location_name,
                self.thumbnail_path,
                self.has_thumbnail,
                self.country,
                self.keywords,
                self.faces_detected,
                self.objects_detected,
                self.colors,
                self.duration,
                self.video_codec,
                self.audio_codec,
                self.bitrate,
                self.frame_rate,
                self.is_favorite.unwrap_or(false),
                Utc::now().to_rfc3339(),
                Utc::now().to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    pub fn delete(pool: &DbPool, hash: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let deleted_rows = conn.execute("DELETE FROM photos WHERE hash_sha256 = ?", [hash])?;
        Ok(deleted_rows > 0)
    }

    pub fn get_cameras(pool: &DbPool) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT camera_make, camera_model FROM photos 
             WHERE camera_make IS NOT NULL AND camera_model IS NOT NULL 
             ORDER BY camera_make, camera_model",
        )?;

        let camera_iter = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut cameras = Vec::new();
        for camera in camera_iter {
            cameras.push(camera?);
        }
        Ok(cameras)
    }

    pub fn get_stats(pool: &DbPool) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Total photos
        let total_photos: i64 =
            conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?;

        // Photos by year
        let mut stmt = conn.prepare(
            "SELECT strftime('%Y', taken_at) as year, COUNT(*) as count 
             FROM photos 
             WHERE taken_at IS NOT NULL 
             GROUP BY year 
             ORDER BY year DESC",
        )?;

        let year_iter = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "year": row.get::<_, String>(0)?,
                "count": row.get::<_, i64>(1)?
            }))
        })?;

        let mut years = Vec::new();
        for year in year_iter {
            years.push(year?);
        }

        Ok(serde_json::json!({
            "total_photos": total_photos,
            "photos_by_year": years
        }))
    }

    pub fn search_photos(
        pool: &DbPool,
        query: &SearchQuery,
        limit: i64,
        offset: i64,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Build the WHERE clause (reusable for both count and data queries)
        let mut where_clause = String::from(" WHERE 1=1");
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref q) = query.q {
            // Handle special type: queries
            if q.starts_with("type:") {
                let media_type = q.strip_prefix("type:").unwrap_or("");
                match media_type {
                    "video" => {
                        where_clause.push_str(" AND mime_type LIKE 'video/%'");
                    }
                    "image" => {
                        where_clause.push_str(" AND mime_type LIKE 'image/%'");
                    }
                    _ => {
                        // Unknown type, fall back to general search
                        where_clause.push_str(" AND (filename LIKE ? OR keywords LIKE ? OR camera_make LIKE ? OR camera_model LIKE ?)");
                        let pattern = format!("%{}%", q);
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern));
                    }
                }
            } else if q.starts_with("is_favorite:") {
                let favorite_value = q.strip_prefix("is_favorite:").unwrap_or("");
                match favorite_value {
                    "true" => {
                        where_clause.push_str(" AND is_favorite = 1");
                    }
                    "false" => {
                        where_clause.push_str(" AND (is_favorite = 0 OR is_favorite IS NULL)");
                    }
                    _ => {
                        // Unknown value, fall back to general search
                        where_clause.push_str(" AND (filename LIKE ? OR keywords LIKE ? OR camera_make LIKE ? OR camera_model LIKE ?)");
                        let pattern = format!("%{}%", q);
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern));
                    }
                }
            } else {
                // General search across multiple fields
                where_clause.push_str(" AND (filename LIKE ? OR keywords LIKE ? OR camera_make LIKE ? OR camera_model LIKE ?)");
                let pattern = format!("%{}%", q);
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern));
            }
        }

        if let Some(ref camera_make) = query.camera_make {
            where_clause.push_str(" AND camera_make LIKE ?");
            params.push(Box::new(format!("%{}%", camera_make)));
        }

        if let Some(ref camera_model) = query.camera_model {
            where_clause.push_str(" AND camera_model LIKE ?");
            params.push(Box::new(format!("%{}%", camera_model)));
        }

        if let Some(year) = query.year {
            where_clause.push_str(" AND strftime('%Y', taken_at) = ?");
            params.push(Box::new(year.to_string()));
        }

        if let Some(month) = query.month {
            where_clause.push_str(" AND strftime('%m', taken_at) = ?");
            params.push(Box::new(format!("{:02}", month)));
        }

        if let Some(ref keywords) = query.keywords {
            where_clause.push_str(" AND keywords LIKE ?");
            params.push(Box::new(format!("%{}%", keywords)));
        }

        if let Some(has_location) = query.has_location {
            if has_location {
                where_clause.push_str(" AND latitude IS NOT NULL AND longitude IS NOT NULL");
            } else {
                where_clause.push_str(" AND (latitude IS NULL OR longitude IS NULL)");
            }
        }

        if let Some(ref country) = query.country {
            where_clause.push_str(" AND country LIKE ?");
            params.push(Box::new(format!("%{}%", country)));
        }

        // Get total count
        let count_sql = format!("SELECT COUNT(*) FROM photos{}", where_clause);
        let mut count_stmt = conn.prepare(&count_sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let total: i64 = count_stmt.query_row(param_refs.as_slice(), |row| row.get(0))?;

        // Get the actual photos
        let sort_field = match sort {
            Some("filename") | Some("name") => "filename",
            Some("camera_make") => "camera_make",
            Some("camera_model") => "camera_model",
            Some("file_size") | Some("size") => "file_size",
            Some("created_at") => "created_at",
            Some("date") => "taken_at",
            _ => "taken_at", // default
        };

        let sort_order = match order {
            Some("asc") => "ASC",
            _ => "DESC", // default
        };

        let data_sql = format!(
            "SELECT * FROM photos{} ORDER BY {} {} LIMIT ? OFFSET ?",
            where_clause, sort_field, sort_order
        );
        params.push(Box::new(limit));
        params.push(Box::new(offset));

        let mut stmt = conn.prepare(&data_sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let photo_iter = stmt.query_map(param_refs.as_slice(), Photo::from_row)?;

        let mut photos = Vec::new();
        for photo in photo_iter {
            photos.push(photo?);
        }

        Ok((photos, total))
    }

    pub fn get_search_suggestions(
        pool: &DbPool,
        _query: Option<&str>,
    ) -> Result<Vec<SearchSuggestion>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut suggestions = Vec::new();

        // Camera makes
        let mut stmt = conn.prepare(
            "SELECT camera_make, COUNT(*) as count
             FROM photos
             WHERE camera_make IS NOT NULL
             GROUP BY camera_make
             ORDER BY count DESC
             LIMIT 10",
        )?;

        let camera_iter = stmt.query_map([], |row| {
            Ok(SearchSuggestion {
                term: row.get::<_, String>(0)?,
                count: row.get::<_, i64>(1)?,
                category: "camera_make".to_string(),
            })
        })?;

        for suggestion in camera_iter {
            suggestions.push(suggestion?);
        }

        // Years
        let mut stmt = conn.prepare(
            "SELECT strftime('%Y', taken_at) as year, COUNT(*) as count
             FROM photos
             WHERE taken_at IS NOT NULL
             GROUP BY year
             ORDER BY count DESC
             LIMIT 10",
        )?;

        let year_iter = stmt.query_map([], |row| {
            Ok(SearchSuggestion {
                term: row.get::<_, String>(0)?,
                count: row.get::<_, i64>(1)?,
                category: "year".to_string(),
            })
        })?;

        for suggestion in year_iter {
            suggestions.push(suggestion?);
        }

        Ok(suggestions)
    }

    pub fn get_timeline_data(pool: &DbPool) -> Result<TimelineData, Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Get min and max dates
        let (min_date, max_date): (Option<String>, Option<String>) = conn.query_row(
            "SELECT MIN(taken_at), MAX(taken_at) FROM photos WHERE taken_at IS NOT NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        // Get photo density by year and month
        let mut stmt = conn.prepare(
            "SELECT
                CAST(strftime('%Y', taken_at) AS INTEGER) as year,
                CAST(strftime('%m', taken_at) AS INTEGER) as month,
                COUNT(*) as count
             FROM photos
             WHERE taken_at IS NOT NULL
             GROUP BY year, month
             ORDER BY year, month",
        )?;

        let density_iter = stmt.query_map([], |row| {
            Ok(TimelineDensity {
                year: row.get(0)?,
                month: row.get(1)?,
                count: row.get(2)?,
            })
        })?;

        let mut density = Vec::new();
        for item in density_iter {
            density.push(item?);
        }

        Ok(TimelineData {
            min_date,
            max_date,
            density,
        })
    }
}

impl From<crate::indexer::ProcessedPhoto> for Photo {
    fn from(processed: crate::indexer::ProcessedPhoto) -> Self {
        Photo {
            hash_sha256: processed
                .hash_sha256
                .expect("ProcessedPhoto must have hash_sha256"),
            file_path: processed.file_path,
            filename: processed.filename,
            file_size: processed.file_size,
            mime_type: processed.mime_type,
            taken_at: processed.taken_at,
            date_modified: processed.date_modified,
            date_indexed: Some(Utc::now()),
            camera_make: processed.camera_make,
            camera_model: processed.camera_model,
            lens_make: processed.lens_make,
            lens_model: processed.lens_model,
            iso: processed.iso,
            aperture: processed.aperture,
            shutter_speed: processed.shutter_speed,
            focal_length: processed.focal_length,
            width: processed.width,
            height: processed.height,
            color_space: processed.color_space,
            white_balance: processed.white_balance,
            exposure_mode: processed.exposure_mode,
            metering_mode: processed.metering_mode,
            orientation: processed.orientation,
            flash_used: processed.flash_used,
            latitude: processed.latitude,
            longitude: processed.longitude,
            location_name: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            country: None,
            keywords: None,
            faces_detected: None,
            objects_detected: None,
            colors: None,
            duration: processed.duration,
            video_codec: processed.video_codec,
            audio_codec: processed.audio_codec,
            bitrate: processed.bitrate,
            frame_rate: processed.frame_rate,
            is_favorite: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
impl Photo {}

#[cfg(test)]
pub fn create_test_db_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    crate::db_pool::create_in_memory_pool()
}

#[cfg(test)]
pub fn create_in_memory_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    crate::db_pool::create_in_memory_pool()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_photo_with_date(hash: &str, filename: &str, taken_at: DateTime<Utc>) -> Photo {
        Photo {
            hash_sha256: hash.to_string(),
            file_path: format!("./test/{}", filename),
            filename: filename.to_string(),
            file_size: 1024,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: Some(taken_at),
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
            width: Some(1920),
            height: Some(1080),
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
            is_favorite: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_get_timeline_data() {
        let pool = create_test_db_pool().unwrap();

        // Create test photos with different dates
        let photo1 = create_test_photo_with_date(
            &"a".repeat(64),
            "photo1.jpg",
            DateTime::parse_from_rfc3339("2010-05-25T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let photo2 = create_test_photo_with_date(
            &"b".repeat(64),
            "photo2.jpg",
            DateTime::parse_from_rfc3339("2010-05-26T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let photo3 = create_test_photo_with_date(
            &"c".repeat(64),
            "photo3.jpg",
            DateTime::parse_from_rfc3339("2011-12-01T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let photo4 = create_test_photo_with_date(
            &"d".repeat(64),
            "photo4.jpg",
            DateTime::parse_from_rfc3339("2024-01-15T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );

        // Insert photos
        photo1.create(&pool).unwrap();
        photo2.create(&pool).unwrap();
        photo3.create(&pool).unwrap();
        photo4.create(&pool).unwrap();

        // Get timeline data
        let timeline = Photo::get_timeline_data(&pool).unwrap();

        // Verify min/max dates
        assert_eq!(
            timeline.min_date,
            Some("2010-05-25T10:00:00+00:00".to_string())
        );
        assert_eq!(
            timeline.max_date,
            Some("2024-01-15T10:00:00+00:00".to_string())
        );

        // Verify density data
        assert_eq!(timeline.density.len(), 3); // 3 unique year-month combinations

        // Check May 2010 (2 photos)
        let may_2010 = timeline
            .density
            .iter()
            .find(|d| d.year == 2010 && d.month == 5)
            .unwrap();
        assert_eq!(may_2010.count, 2);

        // Check December 2011 (1 photo)
        let dec_2011 = timeline
            .density
            .iter()
            .find(|d| d.year == 2011 && d.month == 12)
            .unwrap();
        assert_eq!(dec_2011.count, 1);

        // Check January 2024 (1 photo)
        let jan_2024 = timeline
            .density
            .iter()
            .find(|d| d.year == 2024 && d.month == 1)
            .unwrap();
        assert_eq!(jan_2024.count, 1);
    }

    #[test]
    fn test_get_timeline_data_empty() {
        let pool = create_test_db_pool().unwrap();

        // Get timeline data from empty database
        let timeline = Photo::get_timeline_data(&pool).unwrap();

        // Should return None for dates and empty density
        assert_eq!(timeline.min_date, None);
        assert_eq!(timeline.max_date, None);
        assert_eq!(timeline.density.len(), 0);
    }
}

// CLIP embedding functions

/// Save CLIP embedding for a photo
pub fn save_embedding(
    pool: &DbPool,
    photo_hash: &str,
    embedding: &[f32],
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    // Convert f32 slice to byte slice
    let bytes: Vec<u8> = embedding
        .iter()
        .flat_map(|f| f.to_ne_bytes())
        .collect();

    // Virtual tables (sqlite-vec) don't support INSERT OR REPLACE properly
    // Delete existing entry first, then insert
    conn.execute(
        "DELETE FROM photo_embeddings WHERE photo_hash = ?",
        rusqlite::params![photo_hash],
    )?;

    conn.execute(
        "INSERT INTO photo_embeddings (photo_hash, embedding) VALUES (?, ?)",
        rusqlite::params![photo_hash, bytes],
    )?;
    Ok(())
}

/// Get CLIP embedding for a photo
#[allow(dead_code)]
pub fn get_embedding(
    pool: &DbPool,
    photo_hash: &str,
) -> Result<Option<Vec<f32>>, Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT embedding FROM photo_embeddings WHERE photo_hash = ?")?;

    match stmt.query_row([photo_hash], |row| {
        let bytes: Vec<u8> = row.get(0)?;
        Ok(bytes)
    }) {
        Ok(bytes) => {
            // Convert bytes back to f32 vector
            let floats: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            Ok(Some(floats))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(Box::new(e)),
    }
}

/// Search photos by CLIP embedding similarity
#[allow(dead_code)]
pub fn search_by_clip_embedding(
    pool: &DbPool,
    query_embedding: &[f32],
    limit: i64,
    similarity_threshold: f32,
) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    // Convert query embedding to bytes
    let query_bytes: Vec<u8> = query_embedding
        .iter()
        .flat_map(|f| f.to_ne_bytes())
        .collect();

    // Use sqlite-vec for vector similarity search
    // vec_distance_cosine returns distance (lower is more similar)
    // We want similarity, so we use threshold as max distance
    let mut stmt = conn.prepare(
        "SELECT p.*
         FROM photos p
         JOIN photo_embeddings e ON p.hash_sha256 = e.photo_hash
         WHERE vec_distance_cosine(e.embedding, ?) < ?
         ORDER BY vec_distance_cosine(e.embedding, ?) ASC
         LIMIT ?",
    )?;

    let max_distance = 1.0 - similarity_threshold; // Convert similarity to distance

    let photo_iter = stmt.query_map(
        params![&query_bytes, max_distance, &query_bytes, limit],
        Photo::from_row,
    )?;

    let mut photos = Vec::new();
    for photo in photo_iter {
        photos.push(photo?);
    }

    Ok(photos)
}
