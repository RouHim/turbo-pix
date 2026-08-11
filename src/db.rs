use chrono::{DateTime, NaiveDateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, Row};

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

fn build_general_search_condition() -> &'static str {
    "(filename LIKE ? ESCAPE '\\' OR json_extract(metadata, '$.camera.make') LIKE ? ESCAPE '\\' OR json_extract(metadata, '$.camera.model') LIKE ? ESCAPE '\\' OR json_extract(metadata, '$.location.city') LIKE ? ESCAPE '\\')"
}

/// Escape LIKE wildcards (`%`, `_`) and the escape character itself so user
/// input matches literally. Without this, a query like `IMG_2024` also matches
/// `IMGX2024` (`_` = any single char) and a `%` in the query matches every row
/// (LIKE '%%'). Backslash must be escaped FIRST or the other escapes would
/// produce `\%` sequences that the ESCAPE clause then consumes.
fn escape_like(token: &str) -> String {
    token
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn add_general_search_params(params: &mut Vec<String>, query: &str) {
    let pattern = format!("%{}%", escape_like(query));
    params.push(pattern.clone());
    params.push(pattern.clone());
    params.push(pattern.clone());
    params.push(pattern);
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
    pub fn camera_make(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("make")?.as_str()
    }

    pub fn camera_model(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("model")?.as_str()
    }

    pub fn lens_make(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("lens_make")?.as_str()
    }

    pub fn lens_model(&self) -> Option<&str> {
        self.metadata.get("camera")?.get("lens_model")?.as_str()
    }

    // Settings
    pub fn iso(&self) -> Option<i32> {
        self.metadata
            .get("settings")?
            .get("iso")?
            .as_i64()?
            .try_into()
            .ok()
    }

    pub fn aperture(&self) -> Option<f64> {
        self.metadata.get("settings")?.get("aperture")?.as_f64()
    }

    pub fn shutter_speed(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("shutter_speed")?
            .as_str()
    }

    pub fn focal_length(&self) -> Option<f64> {
        self.metadata.get("settings")?.get("focal_length")?.as_f64()
    }

    pub fn exposure_mode(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("exposure_mode")?
            .as_str()
    }

    pub fn metering_mode(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("metering_mode")?
            .as_str()
    }

    pub fn white_balance(&self) -> Option<&str> {
        self.metadata
            .get("settings")?
            .get("white_balance")?
            .as_str()
    }

    pub fn color_space(&self) -> Option<&str> {
        self.metadata.get("settings")?.get("color_space")?.as_str()
    }

    pub fn flash_used(&self) -> Option<bool> {
        self.metadata.get("settings")?.get("flash_used")?.as_bool()
    }

    // Location
    pub fn latitude(&self) -> Option<f64> {
        self.metadata.get("location")?.get("latitude")?.as_f64()
    }

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

        // Deterministic pagination: `hash_sha256` (unique) breaks ties on the
        // primary sort key — camera bursts share the identical EXIF second,
        // and SQLite's tie order follows scan/rowid order, which shifts when
        // a background rescan inserts/updates rows between page fetches
        // (photos then appear on two pages or get skipped).
        let query_str = format!(
            "SELECT * FROM photos ORDER BY {} {}, hash_sha256 {} LIMIT ? OFFSET ?",
            sort_field, sort_order, sort_order
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
    ///
    /// # Transaction Requirement
    ///
    /// Caller must provide an active transaction: rewriting `hash_sha256` (the parent
    /// PK referenced by `housekeeping_candidates.photo_hash` via `ON DELETE CASCADE`
    /// with no `ON UPDATE`) requires deleting/repainting the stale candidate rows
    /// inside the same transaction (see `image_editor::rotate_image`).
    pub async fn update_with_old_hash(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
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
        .execute(&mut **tx)
        .await?;
        // A stale-snapshot write (the row's hash changed while we were
        // rotating — e.g. a second overlapping rotate request) matches 0
        // rows; committing silently would leave the DB divergent from the
        // file on disk. Fail loudly so the caller aborts the transaction.
        let affected = sqlx::query("SELECT changes()")
            .fetch_one(&mut **tx)
            .await?
            .try_get::<i64, _>(0)?;
        if affected == 0 {
            return Err(format!(
                "Photo with hash {} no longer exists (stale snapshot?) — update matched 0 rows",
                old_hash
            )
            .into());
        }
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
    ///    (the hash is derived from the file PATH, so an in-app rotation — which
    ///    rewrites the bytes — produces a different content hash than the rescan's
    ///    path hash; content changes at the same path keep the hash, so cache
    ///    invalidation is handled by the size+mtime content version in the
    ///    thumbnail/transcode/collage keys, NOT by this branch)
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
        //
        // Capture user state from the row being replaced: the UPSERT below can
        // only preserve is_favorite via COALESCE when the row still exists
        // (same hash). After an in-app rotation the row is keyed by a content
        // hash while the next rescan re-derives the path hash, so the row is
        // deleted before the upsert — without this carry the favorite flag
        // would silently reset on every rotated photo.
        let replaced_favorite: Option<Option<bool>> = sqlx::query_scalar(
            "SELECT is_favorite FROM photos WHERE file_path = ? AND hash_sha256 != ?",
        )
        .bind(&self.file_path)
        .bind(&self.hash_sha256)
        .fetch_optional(&mut **tx)
        .await?;

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
        .bind(
            replaced_favorite
                .flatten()
                .or(self.is_favorite)
                .unwrap_or(false),
        )
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

        if let Some(q) = &query.q {
            // Split on whitespace and AND per-token conditions so combined
            // queries like "sunset is_favorite:true" work. A token with an
            // unknown type:/is_favorite: value falls back to general search.
            let tokens: Vec<&str> = q.split_whitespace().collect();
            let mut i = 0;
            while i < tokens.len() {
                let token = tokens[i];
                if let Some(media_type) = token.strip_prefix("type:") {
                    match media_type {
                        "video" => where_clause.push_str(" AND mime_type LIKE 'video/%'"),
                        "image" => where_clause.push_str(" AND mime_type LIKE 'image/%'"),
                        _ => {
                            // Unknown type, fall back to general search
                            where_clause.push_str(" AND ");
                            where_clause.push_str(build_general_search_condition());
                            add_general_search_params(&mut params, token);
                        }
                    }
                } else if let Some(favorite_value) = token.strip_prefix("is_favorite:") {
                    match favorite_value {
                        "true" => where_clause.push_str(" AND is_favorite = 1"),
                        "false" => {
                            where_clause.push_str(" AND (is_favorite = 0 OR is_favorite IS NULL)");
                        }
                        _ => {
                            // Unknown value, fall back to general search
                            where_clause.push_str(" AND ");
                            where_clause.push_str(build_general_search_condition());
                            add_general_search_params(&mut params, token);
                        }
                    }
                } else if token.starts_with("location:") {
                    // Absorb following words until the next prefix token or
                    // end, so multi-word cities ("location:New York") keep
                    // working.
                    let mut city = token.strip_prefix("location:").unwrap_or("").to_string();
                    while i + 1 < tokens.len()
                        && !tokens[i + 1].starts_with("type:")
                        && !tokens[i + 1].starts_with("is_favorite:")
                        && !tokens[i + 1].starts_with("location:")
                    {
                        i += 1;
                        city.push(' ');
                        city.push_str(tokens[i]);
                    }
                    if city.trim().is_empty() {
                        // Bare "location:" token — no city to match, skip it
                        // (LIKE '%%' would match every row with a city).
                    } else {
                        // Trim once: "location: New York" accumulates a leading
                        // space during absorption that would break the LIKE
                        // pattern ('% New York%' matches nothing).
                        let city = city.trim();
                        where_clause.push_str(
                            " AND json_extract(metadata, '$.location.city') LIKE ? ESCAPE '\\'",
                        );
                        params.push(format!("%{}%", escape_like(city)));
                    }
                } else {
                    // General search across multiple fields (filename + JSON metadata)
                    where_clause.push_str(" AND ");
                    where_clause.push_str(build_general_search_condition());
                    add_general_search_params(&mut params, token);
                }
                i += 1;
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
            "SELECT * FROM photos{} ORDER BY {} {}, hash_sha256 {} LIMIT ? OFFSET ?",
            where_clause, sort_field, sort_order, sort_order
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
            semantic_vector_indexed: processed.semantic_vector_indexed,
            metadata: json!(metadata),
            date_modified: processed.date_modified,
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

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

pub async fn get_photos_needing_geo_resolution(
    pool: &DbPool,
) -> Result<Vec<(String, f64, f64)>, Box<dyn std::error::Error>> {
    let photos: Vec<(String, f64, f64)> = sqlx::query_as(
        "SELECT
            file_path,
            json_extract(metadata, '$.location.latitude') AS latitude,
            json_extract(metadata, '$.location.longitude') AS longitude
         FROM photos
         WHERE geo_location_resolved = 0
           AND json_extract(metadata, '$.location.latitude') IS NOT NULL
         ORDER BY file_path",
    )
    .fetch_all(pool)
    .await?;
    Ok(photos)
}

pub async fn mark_photo_geo_resolved(
    pool: &DbPool,
    file_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query("UPDATE photos SET geo_location_resolved = 1 WHERE file_path = ?")
        .bind(file_path)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_photo_city(
    pool: &DbPool,
    file_path: &str,
    city: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(city) = city {
        sqlx::query("UPDATE photos SET metadata = json_set(metadata, '$.location.city', ?) WHERE file_path = ?")
            .bind(city)
            .bind(file_path)
            .execute(pool)
            .await?;
    }

    mark_photo_geo_resolved(pool, file_path).await
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

    fn create_test_photo_with_metadata(
        filename: &str,
        hash: &str,
        metadata: serde_json::Value,
    ) -> Photo {
        let mut photo = create_test_photo(filename.to_string(), hash.to_string());
        photo.metadata = metadata;
        photo
    }

    async fn read_photo_metadata(pool: &DbPool, file_path: &str) -> serde_json::Value {
        let metadata: String =
            sqlx::query_scalar("SELECT metadata FROM photos WHERE file_path = ?")
                .bind(file_path)
                .fetch_one(pool)
                .await
                .unwrap();

        serde_json::from_str(&metadata).unwrap()
    }

    fn create_search_query(query: &str) -> SearchQuery {
        SearchQuery {
            q: Some(query.to_string()),
            year: None,
            month: None,
        }
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
    async fn test_rotate_db_update_removes_housekeeping_candidate() {
        let pool = create_test_db_pool().await.unwrap();

        // Create a photo and a stale housekeeping candidate referencing its hash
        let photo = create_test_photo_with_date(&"a".repeat(64), "rotate.jpg", Utc::now());
        photo.create(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO housekeeping_candidates (photo_hash, reason, score) VALUES (?, 'test', 0.5)",
        )
        .bind(&photo.hash_sha256)
        .execute(&pool)
        .await
        .unwrap();

        // Simulate rotate_image's DB sequence: delete the stale candidate row and
        // rewrite the PK inside one transaction (AGENTS.md known bug: FK has no
        // ON UPDATE, so a bare PK rewrite fails)
        let mut updated = photo.clone();
        updated.hash_sha256 = "b".repeat(64);
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM housekeeping_candidates WHERE photo_hash = ?")
            .bind(&photo.hash_sha256)
            .execute(&mut *tx)
            .await
            .unwrap();
        updated
            .update_with_old_hash(&mut tx, &photo.hash_sha256)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // Old candidate row is gone; photo lives under the new hash
        let old_candidates: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping_candidates WHERE photo_hash = ?")
                .bind(&photo.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(old_candidates, 0);
        let new_photos: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE hash_sha256 = ?")
                .bind(&updated.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(new_photos, 1);
    }

    #[tokio::test]
    async fn test_create_or_update_preserves_favorite_across_hash_rekey() {
        let pool = create_test_db_pool().await.unwrap();

        // GIVEN: a favorited photo keyed by hash H1 at path rotate.jpg
        let mut photo = create_test_photo_with_date(&"a".repeat(64), "rotate.jpg", Utc::now());
        photo.is_favorite = Some(true);
        photo.create(&pool).await.unwrap();

        // WHEN: the same path is re-keyed under a different hash (the
        // rotate-then-rescan sequence: in-app rotation keys the row by the
        // content hash, the next rescan re-derives the path hash)
        let mut reprocessed = photo.clone();
        reprocessed.hash_sha256 = "b".repeat(64);
        reprocessed.is_favorite = None; // fresh extraction knows nothing about favorites
        let mut tx = pool.begin().await.unwrap();
        reprocessed
            .create_or_update_with_transaction(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // THEN: the favorite flag survived the re-key
        let stored: Option<bool> =
            sqlx::query_scalar("SELECT is_favorite FROM photos WHERE hash_sha256 = ?")
                .bind("b".repeat(64))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, Some(true), "is_favorite must survive a hash re-key");

        // AND: the old row is gone (the DELETE really ran, not just an UPSERT)
        let old_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos WHERE hash_sha256 = ?")
            .bind("a".repeat(64))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            old_rows, 0,
            "re-keyed row must not remain under the old hash"
        );

        // AND: a same-hash rescan keeps the favorite via COALESCE (no re-key)
        let mut rescan = photo.clone();
        rescan.hash_sha256 = "b".repeat(64);
        rescan.is_favorite = None;
        let mut tx = pool.begin().await.unwrap();
        rescan
            .create_or_update_with_transaction(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let stored: Option<bool> =
            sqlx::query_scalar("SELECT is_favorite FROM photos WHERE hash_sha256 = ?")
                .bind("b".repeat(64))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            stored,
            Some(true),
            "same-hash upsert must keep the favorite"
        );
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

    #[tokio::test]
    async fn test_geo_location_resolved_defaults_to_false_and_persists_true() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo(
            "geo-default.jpg".to_string(),
            "geo-default-hash".to_string(),
        );

        photo.create(&pool).await.unwrap();

        let initial_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE hash_sha256 = ?")
                .bind(&photo.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(initial_value, 0);

        sqlx::query("UPDATE photos SET geo_location_resolved = 1 WHERE hash_sha256 = ?")
            .bind(&photo.hash_sha256)
            .execute(&pool)
            .await
            .unwrap();

        let updated_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE hash_sha256 = ?")
                .bind(&photo.hash_sha256)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(updated_value, 1);
    }

    #[tokio::test]
    async fn test_geo_location_resolved_defaults_false_for_multiple_rows() {
        let pool = create_test_db_pool().await.unwrap();

        for index in 0..3 {
            let photo = create_test_photo(
                format!("geo-default-{}.jpg", index),
                format!("geo-default-hash-{}", index),
            );
            photo.create(&pool).await.unwrap();
        }

        let resolved_values: Vec<i64> =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos ORDER BY file_path")
                .fetch_all(&pool)
                .await
                .unwrap();

        assert_eq!(resolved_values, [0, 0, 0]);
    }

    #[tokio::test]
    async fn test_get_photos_needing_geo_resolution() {
        let pool = create_test_db_pool().await.unwrap();
        let unresolved_photo = create_test_photo_with_metadata(
            "needs-geo.jpg",
            "needs-geo-hash",
            json!({
                "location": {
                    "latitude": 52.52,
                    "longitude": 13.405
                }
            }),
        );
        let resolved_photo = create_test_photo_with_metadata(
            "resolved-geo.jpg",
            "resolved-geo-hash",
            json!({
                "location": {
                    "latitude": 48.137,
                    "longitude": 11.575
                }
            }),
        );
        let no_gps_photo = create_test_photo_with_metadata(
            "no-gps.jpg",
            "no-gps-hash",
            json!({
                "camera": {
                    "make": "Canon"
                }
            }),
        );

        unresolved_photo.create(&pool).await.unwrap();
        resolved_photo.create(&pool).await.unwrap();
        no_gps_photo.create(&pool).await.unwrap();

        sqlx::query("UPDATE photos SET geo_location_resolved = 1 WHERE file_path = ?")
            .bind(&resolved_photo.file_path)
            .execute(&pool)
            .await
            .unwrap();

        let photos = get_photos_needing_geo_resolution(&pool).await.unwrap();

        assert_eq!(
            photos,
            vec![(unresolved_photo.file_path.clone(), 52.52, 13.405)]
        );
    }

    #[tokio::test]
    async fn test_mark_photo_geo_resolved() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo_with_metadata(
            "mark-resolved.jpg",
            "mark-resolved-hash",
            json!({
                "location": {
                    "latitude": 52.52,
                    "longitude": 13.405
                }
            }),
        );

        photo.create(&pool).await.unwrap();
        mark_photo_geo_resolved(&pool, &photo.file_path)
            .await
            .unwrap();

        let photos = get_photos_needing_geo_resolution(&pool).await.unwrap();

        assert!(photos.is_empty());
    }

    #[tokio::test]
    async fn test_update_photo_city() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo_with_metadata(
            "city-update.jpg",
            "city-update-hash",
            json!({
                "location": {
                    "latitude": 52.52,
                    "longitude": 13.405
                }
            }),
        );

        photo.create(&pool).await.unwrap();
        update_photo_city(&pool, &photo.file_path, Some("Berlin"))
            .await
            .unwrap();

        let metadata = read_photo_metadata(&pool, &photo.file_path).await;
        let resolved_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE file_path = ?")
                .bind(&photo.file_path)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(metadata["location"]["city"], json!("Berlin"));
        assert_eq!(resolved_value, 1);
    }

    #[tokio::test]
    async fn test_update_photo_city_null() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo_with_metadata(
            "city-update-null.jpg",
            "city-update-null-hash",
            json!({
                "location": {
                    "latitude": 52.52,
                    "longitude": 13.405
                }
            }),
        );

        photo.create(&pool).await.unwrap();
        update_photo_city(&pool, &photo.file_path, None)
            .await
            .unwrap();

        let metadata = read_photo_metadata(&pool, &photo.file_path).await;
        let resolved_value: i64 =
            sqlx::query_scalar("SELECT geo_location_resolved FROM photos WHERE file_path = ?")
                .bind(&photo.file_path)
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(metadata["location"].get("city"), None);
        assert_eq!(resolved_value, 1);
    }

    #[tokio::test]
    async fn test_search_photos_by_city() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "berlin.jpg",
            "berlin-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );

        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:Berlin");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, berlin_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_like_wildcards_are_literal() {
        let pool = create_test_db_pool().await.unwrap();
        // `_` and `%` in user input must match literally: without the ESCAPE
        // clause, `IMG_2024` also matches `IMGX2024` and a `%` matches every
        // row (LIKE '%%').
        let underscore_photo =
            create_test_photo_with_metadata("IMG_2024.jpg", "underscore-hash", json!({}));
        let x_photo = create_test_photo_with_metadata("IMGX2024.jpg", "x-hash", json!({}));
        underscore_photo.create(&pool).await.unwrap();
        x_photo.create(&pool).await.unwrap();

        let query = create_search_query("IMG_2024");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1, "underscore must match literally");
        assert_eq!(photos[0].file_path, underscore_photo.file_path);

        // A literal % must match only rows whose filename contains '%'
        let percent_photo =
            create_test_photo_with_metadata("weird%name.jpg", "percent-hash", json!({}));
        percent_photo.create(&pool).await.unwrap();
        let query = create_search_query("100%");
        let (_, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0, "'100%' must not match every row");

        let query = create_search_query("weird%name");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos[0].file_path, percent_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_like_wildcards_literal_in_location() {
        let pool = create_test_db_pool().await.unwrap();
        let city_photo = create_test_photo_with_metadata(
            "cologne.jpg",
            "cologne-hash",
            json!({
                "location": {
                    "city": "Cologne"
                }
            }),
        );
        city_photo.create(&pool).await.unwrap();

        // `_` in a location token must not widen to a single-char wildcard
        let query = create_search_query("location:Col_gne");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(photos.is_empty());
    }

    #[tokio::test]
    async fn test_search_general_includes_city() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "berlin-general.jpg",
            "berlin-general-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );

        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("Berlin");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, berlin_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_photos_by_city_no_match() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "berlin-no-match.jpg",
            "berlin-no-match-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );

        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:Paris");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 0);
        assert!(photos.is_empty());
    }

    #[tokio::test]
    async fn test_search_combined_general_and_favorite() {
        let pool = create_test_db_pool().await.unwrap();
        let mut fav_photo =
            create_test_photo("sunset-fav.jpg".to_string(), "sunset-fav-hash".to_string());
        fav_photo.is_favorite = Some(true);
        fav_photo.create(&pool).await.unwrap();
        let plain_photo =
            create_test_photo("sunset.jpg".to_string(), "sunset-plain-hash".to_string());
        plain_photo.create(&pool).await.unwrap();

        let query = create_search_query("sunset is_favorite:true");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, fav_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_combined_general_and_type() {
        let pool = create_test_db_pool().await.unwrap();
        let mut video_photo =
            create_test_photo("sunset-vid.mp4".to_string(), "sunset-vid-hash".to_string());
        video_photo.mime_type = Some("video/mp4".to_string());
        video_photo.create(&pool).await.unwrap();
        let image_photo =
            create_test_photo("sunset.jpg".to_string(), "sunset-img-hash".to_string());
        image_photo.create(&pool).await.unwrap();

        let query = create_search_query("sunset type:video");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, video_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_location_multiple_words() {
        let pool = create_test_db_pool().await.unwrap();
        let ny_photo = create_test_photo_with_metadata(
            "new-york.jpg",
            "new-york-hash",
            json!({
                "location": {
                    "city": "New York"
                }
            }),
        );
        ny_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:New York");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, ny_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_location_space_separated_after_colon() {
        let pool = create_test_db_pool().await.unwrap();
        let ny_photo = create_test_photo_with_metadata(
            "ny-space.jpg",
            "ny-space-hash",
            json!({
                "location": {
                    "city": "New York"
                }
            }),
        );
        ny_photo.create(&pool).await.unwrap();

        // "location: New York" (space after the colon) absorbs "New York"
        // with a leading space — the LIKE pattern must be trimmed or it
        // matches nothing.
        let query = create_search_query("location: New York");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos[0].file_path, ny_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_bare_location_token_is_skipped() {
        let pool = create_test_db_pool().await.unwrap();
        let photo = create_test_photo_with_metadata(
            "with-city.jpg",
            "with-city-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );
        photo.create(&pool).await.unwrap();
        // A second photo with a city that does NOT match the general token:
        // under the old LIKE '%%' behavior the bare location: token would
        // match BOTH rows (total 2); with the skip it contributes nothing.
        let other = create_test_photo_with_metadata(
            "other.jpg",
            "other-hash",
            json!({
                "location": {
                    "city": "Hamburg"
                }
            }),
        );
        other.create(&pool).await.unwrap();

        // A bare "location:" token must not filter (previously it emitted
        // LIKE '%%' which matched every row with a city).
        let query = create_search_query("with-city location:");
        let (_, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn test_search_location_combined_with_general() {
        let pool = create_test_db_pool().await.unwrap();
        let berlin_photo = create_test_photo_with_metadata(
            "sunset-berlin.jpg",
            "sunset-berlin-hash",
            json!({
                "location": {
                    "city": "Berlin"
                }
            }),
        );
        berlin_photo.create(&pool).await.unwrap();

        let query = create_search_query("sunset location:Berlin");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, berlin_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_injection_shaped_tokens_are_literal() {
        let pool = create_test_db_pool().await.unwrap();
        let mut fav_photo = create_test_photo("sunset-fav.jpg".to_string(), "fav-hash".to_string());
        fav_photo.is_favorite = Some(true);
        fav_photo.create(&pool).await.unwrap();

        for query in ["is_favorite:true' OR '1'='1", "sunset' OR '1'='1 --"] {
            let (photos, total) =
                Photo::search_photos(&pool, &create_search_query(query), 50, 0, None, None)
                    .await
                    .unwrap();
            assert_eq!(total, 0, "query {query:?} must not match");
            assert!(photos.is_empty(), "query {query:?} must not match");
        }
    }

    #[tokio::test]
    async fn test_search_unknown_type_value_falls_back_to_general() {
        let pool = create_test_db_pool().await.unwrap();
        let raw_photo = create_test_photo("x_type:raw_y.jpg".to_string(), "raw-hash".to_string());
        raw_photo.create(&pool).await.unwrap();
        let plain_photo = create_test_photo("sunset.jpg".to_string(), "plain-hash".to_string());
        plain_photo.create(&pool).await.unwrap();

        let query = create_search_query("type:raw");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();

        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, raw_photo.file_path);
    }

    #[tokio::test]
    async fn test_search_location_absorption_stops_at_prefix_token() {
        let pool = create_test_db_pool().await.unwrap();
        let mut ny_fav_photo = create_test_photo_with_metadata(
            "ny-fav.jpg",
            "ny-fav-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_fav_photo.is_favorite = Some(true);
        ny_fav_photo.create(&pool).await.unwrap();
        let ny_plain_photo = create_test_photo_with_metadata(
            "ny-plain.jpg",
            "ny-plain-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_plain_photo.create(&pool).await.unwrap();
        let mut berlin_fav_photo = create_test_photo_with_metadata(
            "berlin-fav.jpg",
            "berlin-fav-hash",
            json!({ "location": { "city": "Berlin" } }),
        );
        berlin_fav_photo.is_favorite = Some(true);
        berlin_fav_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:New York is_favorite:true");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, ny_fav_photo.file_path);

        let mut ny_video_photo = create_test_photo_with_metadata(
            "ny-video.mp4",
            "ny-video-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_video_photo.mime_type = Some("video/mp4".to_string());
        ny_video_photo.create(&pool).await.unwrap();
        let ny_image_photo = create_test_photo_with_metadata(
            "ny-image.jpg",
            "ny-image-hash",
            json!({ "location": { "city": "New York" } }),
        );
        ny_image_photo.create(&pool).await.unwrap();

        let query = create_search_query("location:New York type:video");
        let (photos, total) = Photo::search_photos(&pool, &query, 50, 0, None, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].file_path, ny_video_photo.file_path);
    }
}
