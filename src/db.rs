use chrono::{DateTime, NaiveDateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;

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
    #[serde(deserialize_with = "deserialize_optional_datetime")]
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
    pub semantic_vector_indexed: Option<bool>,

    // === METADATA (JSON blob) ===
    /// Contains: camera{make,model,lens_make,lens_model}, settings{iso,aperture,...},
    /// location{latitude,longitude}, video{codec,audio_codec,bitrate,frame_rate}
    #[serde(deserialize_with = "deserialize_json_value")]
    pub metadata: serde_json::Value,

    // === SYSTEM TIMESTAMPS ===
    #[serde(deserialize_with = "deserialize_datetime", rename = "file_modified")]
    pub date_modified: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_optional_datetime")]
    pub date_indexed: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "deserialize_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_datetime")]
    pub updated_at: DateTime<Utc>,
}

// Custom deserializers for handling SQLite TEXT -> Rust DateTime conversion
fn deserialize_datetime<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    parse_datetime(&s).ok_or_else(|| serde::de::Error::custom("invalid datetime format"))
}

fn deserialize_optional_datetime<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Deserialize::deserialize(deserializer)?;
    Ok(s.and_then(|s| parse_datetime(&s)))
}

fn deserialize_json_value<'de, D>(deserializer: D) -> Result<serde_json::Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    s.parse()
        .map_err(|_| {
            log::warn!("Failed to parse metadata JSON, using empty object");
            D::Error::custom("invalid JSON")
        })
        .or(Ok(json!({})))
}

fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC3339 first (e.g., "2026-01-04T16:17:10Z")
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            // Try SQLite datetime format (e.g., "2026-01-04 16:17:10")
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
        })
}

impl FromRow<'_, sqlx::sqlite::SqliteRow> for Photo {
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;

        Ok(Photo {
            hash_sha256: row.try_get("hash_sha256")?,
            file_path: row.try_get("file_path")?,
            filename: row.try_get("filename")?,
            file_size: row.try_get("file_size")?,
            mime_type: row.try_get("mime_type")?,
            taken_at: row
                .try_get::<Option<String>, _>("taken_at")?
                .and_then(|s| parse_datetime(&s)),
            width: row.try_get("width")?,
            height: row.try_get("height")?,
            orientation: row.try_get("orientation")?,
            duration: row.try_get("duration")?,
            thumbnail_path: row.try_get("thumbnail_path")?,
            has_thumbnail: row.try_get("has_thumbnail")?,
            blurhash: row.try_get("blurhash")?,
            is_favorite: row.try_get("is_favorite")?,
            semantic_vector_indexed: row.try_get("semantic_vector_indexed")?,
            metadata: row
                .try_get::<String, _>("metadata")?
                .parse()
                .unwrap_or_else(|e| {
                    log::warn!("Failed to parse metadata JSON for photo: {}", e);
                    json!({})
                }),
            date_modified: parse_datetime(&row.try_get::<String, _>("file_modified")?).ok_or_else(
                || sqlx::Error::ColumnDecode {
                    index: "file_modified".to_string(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid datetime",
                    )),
                },
            )?,
            date_indexed: row
                .try_get::<Option<String>, _>("date_indexed")?
                .and_then(|s| parse_datetime(&s)),
            created_at: parse_datetime(&row.try_get::<String, _>("created_at")?).ok_or_else(
                || sqlx::Error::ColumnDecode {
                    index: "created_at".to_string(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid datetime",
                    )),
                },
            )?,
            updated_at: parse_datetime(&row.try_get::<String, _>("updated_at")?).ok_or_else(
                || sqlx::Error::ColumnDecode {
                    index: "updated_at".to_string(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid datetime",
                    )),
                },
            )?,
        })
    }
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

    /// Update photo (convenience wrapper)
    pub async fn update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        self.update_with_transaction(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Create or update photo (convenience wrapper)
    /// Use `batch_write_photos` in production for better performance
    #[cfg(test)]
    pub async fn create_or_update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        self.create_or_update_with_transaction(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_with_pagination(
        pool: &DbPool,
        limit: i64,
        offset: i64,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        // Get total count
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
            .fetch_one(pool)
            .await?;

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
        let query_str = format!(
            "SELECT * FROM photos ORDER BY {} {} LIMIT ? OFFSET ?",
            sort_field, sort_order
        );

        let photos = sqlx::query_as::<_, Photo>(&query_str)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;

        Ok((photos, total))
    }

    pub async fn find_by_hash(
        pool: &DbPool,
        hash: &str,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let photo = sqlx::query_as::<_, Photo>("SELECT * FROM photos WHERE hash_sha256 = ?")
            .bind(hash)
            .fetch_optional(pool)
            .await?;

        Ok(photo)
    }

    /// Check if a photo exists with matching path, size, and modification time
    /// Returns the full Photo if unchanged, None if new/modified
    pub async fn find_unchanged_photo(
        pool: &DbPool,
        file_path: &str,
        file_size: i64,
        date_modified: DateTime<Utc>,
    ) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let photo = sqlx::query_as::<_, Photo>(
            "SELECT * FROM photos WHERE file_path = ? AND file_size = ? AND file_modified = ?",
        )
        .bind(file_path)
        .bind(file_size)
        .bind(date_modified.to_rfc3339())
        .fetch_optional(pool)
        .await?;

        Ok(photo)
    }

    /// Create photo using an existing transaction (for batch operations)
    #[cfg(test)]
    pub async fn create_with_transaction(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            r#"
            INSERT INTO photos (
                hash_sha256, file_path, filename, file_size, mime_type,
                taken_at, width, height, orientation, duration,
                thumbnail_path, has_thumbnail, blurhash, is_favorite, semantic_vector_indexed,
                metadata,
                file_modified, date_indexed, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
            )
            "#,
        )
        .bind(&self.hash_sha256)
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(self.is_favorite.unwrap_or(false))
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(self.metadata.to_string())
        .bind(self.date_modified.to_rfc3339())
        .bind(self.date_indexed.map(|dt| dt.to_rfc3339()))
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Create photo (test helper - use create_with_transaction for production)
    #[cfg(test)]
    pub async fn create(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let mut tx = pool.begin().await?;
        self.create_with_transaction(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Update photo using an existing transaction (for batch operations)
    pub async fn update_with_transaction(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            r#"
            UPDATE photos SET
                file_path = ?, filename = ?, file_size = ?, mime_type = ?,
                taken_at = ?, width = ?, height = ?, orientation = ?, duration = ?,
                thumbnail_path = ?, has_thumbnail = ?, blurhash = ?, is_favorite = ?, semantic_vector_indexed = ?,
                metadata = ?,
                file_modified = ?, updated_at = ?
            WHERE hash_sha256 = ?
            "#,
        )
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(self.is_favorite.unwrap_or(false))
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(self.metadata.to_string())
        .bind(self.date_modified.to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .bind(&self.hash_sha256)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    /// Update photo using old hash in WHERE clause (for operations that change the hash)
    pub async fn update_with_old_hash(
        &self,
        pool: &DbPool,
        old_hash: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        sqlx::query(
            r#"
            UPDATE photos SET
                hash_sha256 = ?,
                file_path = ?, filename = ?, file_size = ?, mime_type = ?,
                taken_at = ?, width = ?, height = ?, orientation = ?, duration = ?,
                thumbnail_path = ?, has_thumbnail = ?, blurhash = ?, is_favorite = ?, semantic_vector_indexed = ?,
                metadata = ?,
                file_modified = ?, updated_at = ?
            WHERE hash_sha256 = ?
            "#,
        )
        .bind(&self.hash_sha256)
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(self.is_favorite.unwrap_or(false))
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(self.metadata.to_string())
        .bind(self.date_modified.to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .bind(old_hash)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Create or update photo using an existing transaction (for batch operations)
    ///
    /// # Transaction Requirement
    ///
    /// **IMPORTANT**: This method MUST be called within an active database transaction.
    /// The operation consists of two separate SQL statements (DELETE + UPSERT) that must
    /// execute atomically to prevent race conditions when the same file_path is processed
    /// concurrently or a file's hash changes between operations.
    ///
    /// # Behavior
    ///
    /// 1. Deletes any existing photo with the same `file_path` but different `hash_sha256`
    ///    (handles the case where a file was modified and its hash changed)
    /// 2. Uses UPSERT to insert if new, or update if `hash_sha256` already exists
    ///
    /// # Safety
    ///
    /// Caller must ensure this is called within a transaction. The `batch_write_photos`
    /// function in `scheduler.rs` demonstrates correct usage.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut tx = pool.begin().await?;
    /// sqlx::query("BEGIN IMMEDIATE").execute(&mut *tx).await?;
    /// for photo in photos {
    ///     photo.create_or_update_with_transaction(&mut tx).await?;
    /// }
    /// tx.commit().await?;
    /// ```
    pub async fn create_or_update_with_transaction(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // First, delete any existing photo with same file_path but different hash
        // This handles the case where a file was modified (hash changed)
        sqlx::query("DELETE FROM photos WHERE file_path = ? AND hash_sha256 != ?")
            .bind(&self.file_path)
            .bind(&self.hash_sha256)
            .execute(&mut **tx)
            .await?;

        // Then use UPSERT to insert or update by hash
        sqlx::query(
            r#"
            INSERT INTO photos (
                hash_sha256, file_path, filename, file_size, mime_type,
                taken_at, width, height, orientation, duration,
                thumbnail_path, has_thumbnail, blurhash, is_favorite, semantic_vector_indexed,
                metadata,
                file_modified, date_indexed, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
            )
            ON CONFLICT(hash_sha256) DO UPDATE SET
                file_path = excluded.file_path,
                filename = excluded.filename,
                file_size = excluded.file_size,
                mime_type = excluded.mime_type,
                taken_at = excluded.taken_at,
                width = excluded.width,
                height = excluded.height,
                orientation = excluded.orientation,
                duration = excluded.duration,
                thumbnail_path = excluded.thumbnail_path,
                has_thumbnail = excluded.has_thumbnail,
                blurhash = excluded.blurhash,
                is_favorite = COALESCE(photos.is_favorite, excluded.is_favorite),
                semantic_vector_indexed = excluded.semantic_vector_indexed,
                metadata = excluded.metadata,
                file_modified = excluded.file_modified,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&self.hash_sha256)
        .bind(&self.file_path)
        .bind(&self.filename)
        .bind(self.file_size)
        .bind(&self.mime_type)
        .bind(self.taken_at.map(|dt| dt.to_rfc3339()))
        .bind(self.width)
        .bind(self.height)
        .bind(self.orientation)
        .bind(self.duration)
        .bind(&self.thumbnail_path)
        .bind(self.has_thumbnail)
        .bind(&self.blurhash)
        .bind(self.is_favorite.unwrap_or(false))
        .bind(self.semantic_vector_indexed.unwrap_or(false))
        .bind(self.metadata.to_string())
        .bind(self.date_modified.to_rfc3339())
        .bind(self.date_indexed.map(|dt| dt.to_rfc3339()))
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub async fn search_photos(
        pool: &DbPool,
        query: &SearchQuery,
        limit: i64,
        offset: i64,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        // Build the WHERE clause (reusable for both count and data queries)
        let mut where_clause = String::from(" WHERE 1=1");
        let mut params: Vec<String> = Vec::new();

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
                        params.push(pattern.clone());
                        params.push(pattern.clone());
                        params.push(pattern);
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
                        params.push(pattern.clone());
                        params.push(pattern.clone());
                        params.push(pattern);
                    }
                }
            } else {
                // General search across multiple fields (filename + JSON metadata)
                where_clause.push_str(" AND (filename LIKE ? OR json_extract(metadata, '$.camera.make') LIKE ? OR json_extract(metadata, '$.camera.model') LIKE ?)");
                let pattern = format!("%{}%", q);
                params.push(pattern.clone());
                params.push(pattern.clone());
                params.push(pattern);
            }
        }

        if let Some(year) = query.year {
            where_clause.push_str(" AND strftime('%Y', taken_at) = ?");
            params.push(year.to_string());
        }

        if let Some(month) = query.month {
            where_clause.push_str(" AND strftime('%m', taken_at) = ?");
            params.push(format!("{:02}", month));
        }

        // Get total count
        let count_sql = format!("SELECT COUNT(*) FROM photos{}", where_clause);
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        for param in &params {
            count_query = count_query.bind(param);
        }
        let total = count_query.fetch_one(pool).await?;

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

        let mut data_query = sqlx::query_as::<_, Photo>(&data_sql);
        for param in &params {
            data_query = data_query.bind(param);
        }
        data_query = data_query.bind(limit).bind(offset);

        let photos = data_query.fetch_all(pool).await?;

        Ok((photos, total))
    }

    pub async fn get_timeline_data(
        pool: &DbPool,
    ) -> Result<TimelineData, Box<dyn std::error::Error>> {
        // Get min and max dates
        let (min_date, max_date): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT MIN(taken_at), MAX(taken_at) FROM photos WHERE taken_at IS NOT NULL",
        )
        .fetch_one(pool)
        .await?;

        // Get photo density by year and month
        let density: Vec<TimelineDensity> = sqlx::query_as(
            "SELECT
                CAST(strftime('%Y', taken_at) AS INTEGER) as year,
                CAST(strftime('%m', taken_at) AS INTEGER) as month,
                COUNT(*) as count
             FROM photos
             WHERE taken_at IS NOT NULL
             GROUP BY year, month
             ORDER BY year, month",
        )
        .fetch_all(pool)
        .await?;

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
            semantic_vector_indexed: Some(false), // Phase 1 only indexes metadata
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
pub async fn create_test_db_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    crate::db_pool::create_in_memory_pool().await
}

#[cfg(test)]
pub async fn create_in_memory_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    crate::db_pool::create_in_memory_pool().await
}

/// Get all photo file paths from the database
#[cfg(test)]
pub async fn get_all_photo_paths(pool: &DbPool) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let paths: Vec<String> = sqlx::query_scalar("SELECT file_path FROM photos ORDER BY file_path")
        .fetch_all(pool)
        .await?;
    Ok(paths)
}

/// Get file paths of photos that need semantic vector indexing (Phase 2)
pub async fn get_paths_needing_semantic_indexing(
    pool: &DbPool,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let paths: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM photos WHERE semantic_vector_indexed = 0 OR semantic_vector_indexed IS NULL ORDER BY file_path"
    )
    .fetch_all(pool)
    .await?;
    Ok(paths)
}

/// Mark a photo as semantically indexed
pub async fn mark_photo_as_semantically_indexed(
    pool: &DbPool,
    file_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query("UPDATE photos SET semantic_vector_indexed = 1 WHERE file_path = ?")
        .bind(file_path)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

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
            semantic_vector_indexed: Some(false),
            metadata: json!({}), // Empty metadata for tests
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn create_test_photo(filename: String, hash: String) -> Photo {
        // Ensure hash is 64 characters for SHA256
        let hash_64 = if hash.len() < 64 {
            format!("{:0<64}", hash)
        } else {
            hash
        };
        create_test_photo_with_date(&hash_64, &filename, Utc::now())
    }
    #[tokio::test]
    async fn test_get_timeline_data() {
        let pool = create_test_db_pool().await.unwrap();

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
        photo1.create(&pool).await.unwrap();
        photo2.create(&pool).await.unwrap();
        photo3.create(&pool).await.unwrap();
        photo4.create(&pool).await.unwrap();

        // Get timeline data
        let timeline = Photo::get_timeline_data(&pool).await.unwrap();

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

    #[tokio::test]
    async fn test_get_timeline_data_empty() {
        let pool = create_test_db_pool().await.unwrap();

        // Get timeline data from empty database
        let timeline = Photo::get_timeline_data(&pool).await.unwrap();

        // Should return None for dates and empty density
        assert_eq!(timeline.min_date, None);
        assert_eq!(timeline.max_date, None);
        assert_eq!(timeline.density.len(), 0);
    }

    #[tokio::test]
    async fn test_transaction_rollback_on_constraint_violation() {
        let pool = create_test_db_pool().await.unwrap();

        // Create first photo
        let photo1 = create_test_photo("test1.jpg".to_string(), "abc123".to_string());
        photo1.create(&pool).await.unwrap();

        // Verify photo exists
        let found = Photo::find_by_hash(&pool, &photo1.hash_sha256)
            .await
            .unwrap();
        assert!(found.is_some());

        // Attempt to create photo with duplicate hash in a transaction
        let mut tx = pool.begin().await.unwrap();
        let photo2 = create_test_photo("test2.jpg".to_string(), "abc123".to_string()); // Same hash
        let result = photo2.create_with_transaction(&mut tx).await;

        // Should fail due to PRIMARY KEY constraint
        assert!(result.is_err());

        // Rollback transaction (or let it drop)
        drop(tx);

        // Verify database is still consistent - only one photo exists
        let all_photos = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = all_photos.get("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_transaction_atomicity() {
        let pool = create_test_db_pool().await.unwrap();

        // Create multiple photos in a transaction
        let photos = vec![
            create_test_photo("test1.jpg".to_string(), "hash1".to_string()),
            create_test_photo("test2.jpg".to_string(), "hash2".to_string()),
            create_test_photo("test3.jpg".to_string(), "hash3".to_string()),
        ];

        // Test 1: Successful transaction - all photos committed
        let mut tx = pool.begin().await.unwrap();
        for photo in &photos {
            photo.create_with_transaction(&mut tx).await.unwrap();
        }
        tx.commit().await.unwrap();

        // Verify all photos were committed
        let count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = count.get("count");
        assert_eq!(count, 3, "All photos should be visible after commit");

        // Test 2: Failed transaction - no photos should be added
        let more_photos = vec![
            create_test_photo("test4.jpg".to_string(), "hash4".to_string()),
            create_test_photo("test5.jpg".to_string(), "hash1".to_string()), // Duplicate hash - will fail
        ];

        let mut tx2 = pool.begin().await.unwrap();
        let result = async {
            for photo in &more_photos {
                photo.create_with_transaction(&mut tx2).await?;
            }
            tx2.commit().await?;
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;

        // Transaction should fail due to duplicate hash
        assert!(result.is_err());

        // Verify count is still 3 (rollback worked)
        let final_count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let final_count: i64 = final_count.get("count");
        assert_eq!(
            final_count, 3,
            "Count should remain 3 after failed transaction"
        );
    }

    #[tokio::test]
    async fn test_transaction_update_and_rollback() {
        let pool = create_test_db_pool().await.unwrap();

        // Create initial photo
        let mut photo = create_test_photo("test.jpg".to_string(), "hash123".to_string());
        photo.create(&pool).await.unwrap();

        // Verify initial state
        let original_filename = photo.filename.clone();

        // Start transaction and update photo
        let mut tx = pool.begin().await.unwrap();
        photo.filename = "updated.jpg".to_string();
        photo.update_with_transaction(&mut tx).await.unwrap();

        // Rollback transaction
        drop(tx);

        // Verify photo was NOT updated (rollback worked)
        let found = Photo::find_by_hash(&pool, &photo.hash_sha256)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            found.filename, original_filename,
            "Photo should not be updated after rollback"
        );
    }

    #[tokio::test]
    async fn test_concurrent_writes_consistency() {
        let pool = create_test_db_pool().await.unwrap();

        // Create two photos concurrently
        let photo1 = create_test_photo("test1.jpg".to_string(), "hash1".to_string());
        let photo2 = create_test_photo("test2.jpg".to_string(), "hash2".to_string());

        // Both should succeed since they have different hashes
        let result1 = photo1.create(&pool).await;
        let result2 = photo2.create(&pool).await;

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        // Verify both photos exist
        let count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = count.get("count");
        assert_eq!(count, 2, "Both photos should be created");
    }

    #[tokio::test]
    async fn test_batch_transaction_consistency() {
        let pool = create_test_db_pool().await.unwrap();

        // Create 100 photos in a single transaction to test batch performance
        let mut tx = pool.begin().await.unwrap();

        for i in 0..100 {
            let photo = create_test_photo(
                format!("test_{}.jpg", i),
                format!("{:064}", i), // Generate unique 64-char hash by padding number
            );
            photo.create_with_transaction(&mut tx).await.unwrap();
        }

        // Commit all at once
        tx.commit().await.unwrap();

        // Verify all 100 photos were created
        let count = sqlx::query("SELECT COUNT(*) as count FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        let count: i64 = count.get("count");
        assert_eq!(count, 100, "All 100 photos should be created");
    }
}
