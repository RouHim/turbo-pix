use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{params, Result as SqlResult, Row};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use crate::db_pool::{create_db_pool, delete_orphaned_photos, vacuum_database, DbPool};
pub use crate::db_types::{SearchQuery, TimelineData, TimelineDensity};

/// Photo entity with metadata stored as JSON
/// Breaking change: All EXIF/camera/location/video metadata moved to `metadata` JSON field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    // === CORE IDENTIFICATION ===
    pub hash_sha256: String,
    pub file_path: String,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: Option<String>,

    // === COMPUTATIONAL (used in application logic) ===
    pub taken_at: Option<DateTime<Utc>>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub orientation: Option<i32>,
    pub duration: Option<f64>, // Video duration in seconds

    // === UI STATE ===
    pub thumbnail_path: Option<String>,
    pub has_thumbnail: Option<bool>,
    pub blurhash: Option<String>,
    pub is_favorite: Option<bool>,

    // === METADATA (JSON blob) ===
    /// Contains: camera{make,model,lens_make,lens_model}, settings{iso,aperture,...},
    /// location{latitude,longitude}, video{codec,audio_codec,bitrate,frame_rate}
    pub metadata: serde_json::Value,

    // === SYSTEM TIMESTAMPS ===
    pub date_modified: DateTime<Utc>,
    pub date_indexed: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Photo {
    // ===== METADATA ACCESSORS (for Rust code) =====
    // Frontend reads metadata.* directly from JSON
    // These are public API methods - not all are used internally yet

    // Camera
    #[allow(dead_code)]
    pub fn camera_make(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("make")?.as_str()
    }

    #[allow(dead_code)]
    pub fn camera_model(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("model")?.as_str()
    }

    #[allow(dead_code)]
    pub fn lens_make(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("lens_make")?.as_str()
    }

    #[allow(dead_code)]
    pub fn lens_model(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("lens_model")?.as_str()
    }

    // Settings
    #[allow(dead_code)]
    pub fn iso(&self) -> Option<i32> {
        self.metadata
            .get("settings")?
            .get("iso")?
            .as_i64()?
            .try_into()
            .ok()
    }

    #[allow(dead_code)]
    pub fn aperture(&self) -> Option<f64> {
        self.metadata.get("settings")?.get("aperture")?.as_f64()
    }

    #[allow(dead_code)]
    pub fn shutter_speed(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("shutter_speed")?
            .as_str()
    }

    #[allow(dead_code)]
    pub fn focal_length(&self) -> Option<f64> {
        self.metadata.get("settings")?.get("focal_length")?.as_f64()
    }

    #[allow(dead_code)]
    pub fn exposure_mode(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("exposure_mode")?
            .as_str()
    }

    #[allow(dead_code)]
    pub fn metering_mode(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("metering_mode")?
            .as_str()
    }

    #[allow(dead_code)]
    pub fn white_balance(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("white_balance")?
            .as_str()
    }

    #[allow(dead_code)]
    pub fn color_space(&self) -> Option<&str> {
        self.metadata.get("settings")?.get("color_space")?.as_str()
    }

    #[allow(dead_code)]
    pub fn flash_used(&self) -> Option<bool> {
        self.metadata.get("settings")?.get("flash_used")?.as_bool()
    }

    // Location
    #[allow(dead_code)]
    pub fn latitude(&self) -> Option<f64> {
        self.metadata.get("location")?.get("latitude")?.as_f64()
    }

    #[allow(dead_code)]
    pub fn longitude(&self) -> Option<f64> {
        self.metadata.get("location")?.get("longitude")?.as_f64()
    }

    // Video
    pub fn video_codec(&self) -> Option<&str> {
        self.metadata.get("video")?.get("codec")?.as_str()
    }

    pub fn audio_codec(&self) -> Option<&str> {
        self.metadata.get("video")?.get("audio_codec")?.as_str()
    }

    pub fn bitrate(&self) -> Option<i32> {
        self.metadata
            .get("video")?
            .get("bitrate")?
            .as_i64()?
            .try_into()
            .ok()
    }

    pub fn frame_rate(&self) -> Option<f64> {
        self.metadata.get("video")?.get("frame_rate")?.as_f64()
    }

    // ===== DATABASE OPERATIONS =====

    pub fn from_row(row: &Row) -> SqlResult<Self> {
        Ok(Photo {
            hash_sha256: row.get(0)?,
            file_path: row.get(1)?,
            filename: row.get(2)?,
            file_size: row.get(3)?,
            mime_type: row.get(4)?,
            taken_at: row
                .get::<_, Option<String>>(5)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            width: row.get(6)?,
            height: row.get(7)?,
            orientation: row.get(8)?,
            duration: row.get(9)?,
            thumbnail_path: row.get(10)?,
            has_thumbnail: row.get(11)?,
            blurhash: row.get(12)?,
            is_favorite: row.get(13)?,
            metadata: row
                .get::<_, String>(14)?
                .parse()
                .unwrap_or_else(|_| json!({})),
            date_modified: DateTime::parse_from_rfc3339(&row.get::<_, String>(15)?)
                .unwrap()
                .with_timezone(&Utc),
            date_indexed: row
                .get::<_, Option<String>>(16)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            created_at: {
                let datetime_str = row.get::<_, String>(17)?;
                if datetime_str.contains('T') {
                    DateTime::parse_from_rfc3339(&datetime_str)
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                17,
                                "created_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .with_timezone(&Utc)
                } else {
                    NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                17,
                                "created_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .and_utc()
                }
            },
            updated_at: {
                let datetime_str = row.get::<_, String>(18)?;
                if datetime_str.contains('T') {
                    DateTime::parse_from_rfc3339(&datetime_str)
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                18,
                                "updated_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .with_timezone(&Utc)
                } else {
                    NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                18,
                                "updated_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .and_utc()
                }
            },
        })
    }

    /// Update photo fields from extracted metadata
    /// Preserves existing fields that are not part of the extracted metadata
    pub fn update_from_extracted(&mut self, extracted: crate::metadata_extractor::PhotoMetadata) {
        // Update computational fields
        self.taken_at = extracted.taken_at;
        self.width = extracted.width.map(|w| w as i32);
        self.height = extracted.height.map(|h| h as i32);
        self.orientation = extracted.orientation;
        self.duration = extracted.duration;

        // Build metadata JSON from extracted fields
        self.metadata = json!({
            "camera": {
                "make": extracted.camera_make,
                "model": extracted.camera_model,
                "lens_make": extracted.lens_make,
                "lens_model": extracted.lens_model,
            },
            "settings": {
                "iso": extracted.iso,
                "aperture": extracted.aperture,
                "shutter_speed": extracted.shutter_speed,
                "focal_length": extracted.focal_length,
                "color_space": extracted.color_space,
                "white_balance": extracted.white_balance,
                "exposure_mode": extracted.exposure_mode,
                "metering_mode": extracted.metering_mode,
                "flash_used": extracted.flash_used,
            },
            "location": {
                "latitude": extracted.latitude,
                "longitude": extracted.longitude,
            },
            "video": {
                "codec": extracted.video_codec,
                "audio_codec": extracted.audio_codec,
                "bitrate": extracted.bitrate,
                "frame_rate": extracted.frame_rate,
            }
        });

        // Update timestamp
        self.updated_at = Utc::now();
    }

    pub fn update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute(
            r#"
            UPDATE photos SET
                file_path = ?, filename = ?, file_size = ?, mime_type = ?,
                taken_at = ?, width = ?, height = ?, orientation = ?, duration = ?,
                thumbnail_path = ?, has_thumbnail = ?, blurhash = ?, is_favorite = ?,
                metadata = ?,
                file_modified = ?, updated_at = ?
            WHERE hash_sha256 = ?
            "#,
            params![
                self.file_path,
                self.filename,
                self.file_size,
                self.mime_type,
                self.taken_at.map(|dt| dt.to_rfc3339()),
                self.width,
                self.height,
                self.orientation,
                self.duration,
                self.thumbnail_path,
                self.has_thumbnail,
                self.blurhash,
                self.is_favorite.unwrap_or(false),
                self.metadata.to_string(),
                self.date_modified.to_rfc3339(),
                Utc::now().to_rfc3339(),
                self.hash_sha256,
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

    /// Check if a photo exists with matching path, size, and modification time
    /// Returns the full Photo if unchanged, None if new/modified
    pub fn find_unchanged_photo(
        pool: &DbPool,
        file_path: &str,
        file_size: i64,
        date_modified: DateTime<Utc>,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        let mut stmt = conn.prepare(
            "SELECT * FROM photos WHERE file_path = ? AND file_size = ? AND file_modified = ?",
        )?;

        match stmt.query_row(
            params![file_path, file_size, date_modified.to_rfc3339()],
            Photo::from_row,
        ) {
            Ok(photo) => Ok(Some(photo)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    pub fn create(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        conn.execute(
            r#"
            INSERT INTO photos (
                hash_sha256, file_path, filename, file_size, mime_type,
                taken_at, width, height, orientation, duration,
                thumbnail_path, has_thumbnail, blurhash, is_favorite,
                metadata,
                file_modified, date_indexed, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
            )
            "#,
            params![
                self.hash_sha256,
                self.file_path,
                self.filename,
                self.file_size,
                self.mime_type,
                self.taken_at.map(|dt| dt.to_rfc3339()),
                self.width,
                self.height,
                self.orientation,
                self.duration,
                self.thumbnail_path,
                self.has_thumbnail,
                self.blurhash,
                self.is_favorite.unwrap_or(false),
                self.metadata.to_string(),
                self.date_modified.to_rfc3339(),
                self.date_indexed.map(|dt| dt.to_rfc3339()),
                Utc::now().to_rfc3339(),
                Utc::now().to_rfc3339(),
            ],
        )?;

        Ok(())
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
                        where_clause.push_str(" AND (filename LIKE ? OR json_extract(metadata, '$.camera.make') LIKE ? OR json_extract(metadata, '$.camera.model') LIKE ?)");
                        let pattern = format!("%{}%", q);
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
                        where_clause.push_str(" AND (filename LIKE ? OR json_extract(metadata, '$.camera.make') LIKE ? OR json_extract(metadata, '$.camera.model') LIKE ?)");
                        let pattern = format!("%{}%", q);
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern.clone()));
                        params.push(Box::new(pattern));
                    }
                }
            } else {
                // General search across multiple fields (filename + JSON metadata)
                where_clause.push_str(" AND (filename LIKE ? OR json_extract(metadata, '$.camera.make') LIKE ? OR json_extract(metadata, '$.camera.model') LIKE ?)");
                let pattern = format!("%{}%", q);
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern));
            }
        }

        if let Some(year) = query.year {
            where_clause.push_str(" AND strftime('%Y', taken_at) = ?");
            params.push(Box::new(year.to_string()));
        }

        if let Some(month) = query.month {
            where_clause.push_str(" AND strftime('%m', taken_at) = ?");
            params.push(Box::new(format!("{:02}", month)));
        }

        // Get total count
        let count_sql = format!("SELECT COUNT(*) FROM photos{}", where_clause);
        let mut count_stmt = conn.prepare(&count_sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let total: i64 = count_stmt.query_row(param_refs.as_slice(), |row| row.get(0))?;

        // Get the actual photos
        let sort_field = match sort {
            Some("filename") | Some("name") => "filename",
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
        // Build metadata JSON from ProcessedPhoto fields
        let mut camera = serde_json::Map::new();
        if let Some(make) = processed.camera_make {
            camera.insert("make".to_string(), json!(make));
        }
        if let Some(model) = processed.camera_model {
            camera.insert("model".to_string(), json!(model));
        }
        if let Some(lens_make) = processed.lens_make {
            camera.insert("lens_make".to_string(), json!(lens_make));
        }
        if let Some(lens_model) = processed.lens_model {
            camera.insert("lens_model".to_string(), json!(lens_model));
        }

        let mut settings = serde_json::Map::new();
        if let Some(iso) = processed.iso {
            settings.insert("iso".to_string(), json!(iso));
        }
        if let Some(aperture) = processed.aperture {
            settings.insert("aperture".to_string(), json!(aperture));
        }
        if let Some(shutter_speed) = processed.shutter_speed {
            settings.insert("shutter_speed".to_string(), json!(shutter_speed));
        }
        if let Some(focal_length) = processed.focal_length {
            settings.insert("focal_length".to_string(), json!(focal_length));
        }
        if let Some(exposure_mode) = processed.exposure_mode {
            settings.insert("exposure_mode".to_string(), json!(exposure_mode));
        }
        if let Some(metering_mode) = processed.metering_mode {
            settings.insert("metering_mode".to_string(), json!(metering_mode));
        }
        if let Some(white_balance) = processed.white_balance {
            settings.insert("white_balance".to_string(), json!(white_balance));
        }
        if let Some(color_space) = processed.color_space {
            settings.insert("color_space".to_string(), json!(color_space));
        }
        if let Some(flash_used) = processed.flash_used {
            settings.insert("flash_used".to_string(), json!(flash_used));
        }

        let mut location = serde_json::Map::new();
        if let Some(lat) = processed.latitude {
            location.insert("latitude".to_string(), json!(lat));
        }
        if let Some(lng) = processed.longitude {
            location.insert("longitude".to_string(), json!(lng));
        }

        let mut video = serde_json::Map::new();
        if let Some(codec) = processed.video_codec {
            video.insert("codec".to_string(), json!(codec));
        }
        if let Some(audio_codec) = processed.audio_codec {
            video.insert("audio_codec".to_string(), json!(audio_codec));
        }
        if let Some(bitrate) = processed.bitrate {
            video.insert("bitrate".to_string(), json!(bitrate));
        }
        if let Some(frame_rate) = processed.frame_rate {
            video.insert("frame_rate".to_string(), json!(frame_rate));
        }

        let mut metadata = serde_json::Map::new();
        if !camera.is_empty() {
            metadata.insert("camera".to_string(), json!(camera));
        }
        if !settings.is_empty() {
            metadata.insert("settings".to_string(), json!(settings));
        }
        if !location.is_empty() {
            metadata.insert("location".to_string(), json!(location));
        }
        if !video.is_empty() {
            metadata.insert("video".to_string(), json!(video));
        }

        Photo {
            hash_sha256: processed
                .hash_sha256
                .expect("ProcessedPhoto must have hash_sha256"),
            file_path: processed.file_path,
            filename: processed.filename,
            file_size: processed.file_size,
            mime_type: processed.mime_type,
            taken_at: processed.taken_at,
            width: processed.width,
            height: processed.height,
            orientation: processed.orientation,
            duration: processed.duration,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: processed.blurhash,
            is_favorite: None,
            metadata: json!(metadata),
            date_modified: processed.date_modified,
            date_indexed: Some(Utc::now()),
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
pub fn get_all_photo_paths(pool: &DbPool) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT file_path FROM photos ORDER BY file_path")?;
    let paths = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(paths)
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
            width: Some(1920),
            height: Some(1080),
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: None,
            metadata: json!({}), // Empty metadata for tests
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
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
