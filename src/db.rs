use chrono::{DateTime, NaiveDateTime, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, Result as SqlResult, Row};
use serde::{Deserialize, Serialize};
use tracing::info;

// Type aliases
pub type DbPool = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;

// Search related structs
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub keywords: Option<String>,
    pub has_location: Option<bool>,
    pub country: Option<String>,
    pub limit: Option<u32>,
    pub page: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchSuggestion {
    pub term: String,
    pub count: i64,
    pub category: String,
}

// Schema definitions
pub const PHOTOS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS photos (
    hash_sha256 TEXT PRIMARY KEY NOT NULL CHECK(length(hash_sha256) = 64),
    file_path TEXT NOT NULL UNIQUE,
    filename TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    mime_type TEXT,
    taken_at DATETIME,
    file_modified DATETIME NOT NULL,
    date_indexed DATETIME,
    camera_make TEXT,
    camera_model TEXT,
    lens_make TEXT,
    lens_model TEXT,
    iso INTEGER,
    aperture REAL,
    shutter_speed TEXT,
    focal_length REAL,
    width INTEGER,
    height INTEGER,
    color_space TEXT,
    white_balance TEXT,
    exposure_mode TEXT,
    metering_mode TEXT,
    orientation INTEGER,
    flash_used BOOLEAN,
    latitude REAL,
    longitude REAL,
    location_name TEXT,
    thumbnail_path TEXT,
    has_thumbnail BOOLEAN,
    country TEXT,
    keywords TEXT,
    faces_detected TEXT,
    objects_detected TEXT,
    colors TEXT,
    duration REAL, -- Video duration in seconds
    video_codec TEXT, -- Video codec (e.g., "h264", "h265")
    audio_codec TEXT, -- Audio codec (e.g., "aac", "mp3")
    bitrate INTEGER, -- Bitrate in kbps
    frame_rate REAL, -- Frame rate for videos
    is_favorite BOOLEAN DEFAULT FALSE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
) WITHOUT ROWID;
"#;

pub const SCHEMA_SQL: &[&str] = &[
    PHOTOS_TABLE,
    "CREATE INDEX IF NOT EXISTS idx_photos_file_path ON photos(file_path);",
    "CREATE INDEX IF NOT EXISTS idx_photos_taken_at ON photos(taken_at);",
    "CREATE INDEX IF NOT EXISTS idx_photos_camera_make ON photos(camera_make);",
    "CREATE INDEX IF NOT EXISTS idx_photos_camera_model ON photos(camera_model);",
    "CREATE INDEX IF NOT EXISTS idx_photos_file_modified ON photos(file_modified);",
    "CREATE INDEX IF NOT EXISTS idx_photos_keywords ON photos(keywords);",
    "CREATE INDEX IF NOT EXISTS idx_photos_faces_detected ON photos(faces_detected);",
    "CREATE INDEX IF NOT EXISTS idx_photos_objects_detected ON photos(objects_detected);",
    "CREATE INDEX IF NOT EXISTS idx_photos_colors ON photos(colors);",
    "CREATE INDEX IF NOT EXISTS idx_photos_is_favorite ON photos(is_favorite);",
];

// Connection pool functions
pub fn create_db_pool(database_path: &str) -> Result<DbPool, Box<dyn std::error::Error>> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = std::path::Path::new(database_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let manager = SqliteConnectionManager::file(database_path);
    let pool = Pool::new(manager)?;

    // Initialize schema and configure pragmas on a connection from the pool
    // These pragmas improve concurrency and set a sensible busy timeout.
    {
        let conn = pool.get()?;
        // Set WAL mode (database-level), reasonable sync, keep temp tables in memory,
        // and set a busy timeout so that transient locks are waited on instead of failing immediately.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;",
        )?;
        initialize_schema(&conn)?;
    }

    Ok(pool)
}

pub fn initialize_schema(conn: &Connection) -> SqlResult<()> {
    for sql in SCHEMA_SQL {
        conn.execute(sql, [])?;
    }
    Ok(())
}

// Utility functions
#[allow(dead_code)]
pub fn get_all_photo_paths(pool: &DbPool) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT file_path FROM photos")?;
    let photo_iter = stmt.query_map([], |row| row.get::<_, String>(0))?;

    let mut paths = Vec::new();
    for path in photo_iter {
        paths.push(path?);
    }
    Ok(paths)
}

#[allow(dead_code)]
pub fn needs_update(
    pool: &DbPool,
    file_path: &str,
    file_modified: &DateTime<Utc>,
) -> Result<bool, Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT file_modified FROM photos WHERE file_path = ?")?;

    match stmt.query_row([file_path], |row| {
        let db_modified_str: String = row.get(0)?;
        let db_modified = DateTime::parse_from_rfc3339(&db_modified_str)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
            .with_timezone(&Utc);
        Ok(db_modified)
    }) {
        Ok(db_modified) => Ok(file_modified > &db_modified),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true), // File not in DB, needs insert
        Err(e) => Err(Box::new(e)),
    }
}

pub fn delete_orphaned_photos(
    pool: &DbPool,
    existing_paths: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    if existing_paths.is_empty() {
        // If no existing paths, delete all photos
        conn.execute("DELETE FROM photos", [])?;
        info!("Deleted all photos from database (no files found)");
        return Ok(());
    }

    // Create placeholders for the IN clause
    let placeholders = existing_paths
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "DELETE FROM photos WHERE file_path NOT IN ({})",
        placeholders
    );

    let params: Vec<&dyn rusqlite::ToSql> = existing_paths
        .iter()
        .map(|p| p as &dyn rusqlite::ToSql)
        .collect();

    let deleted_count = conn.execute(&sql, params.as_slice())?;
    info!("Deleted {} orphaned photos from database", deleted_count);

    Ok(())
}

pub fn vacuum_database(pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    conn.execute("VACUUM", [])?;
    info!("Database vacuum completed");
    Ok(())
}

// Main Photo struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub hash_sha256: String, // Now the primary key - always present
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

        // Try to find existing photo by hash
        let existing = conn.query_row(
            "SELECT hash_sha256 FROM photos WHERE hash_sha256 = ?",
            [&self.hash_sha256],
            |row| row.get::<_, String>(0),
        );

        if existing.is_ok() {
            // Photo exists, update it
            self.update(pool)
        } else {
            // Create new photo
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
    create_in_memory_pool()
}

#[cfg(test)]
pub fn create_in_memory_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    let manager = SqliteConnectionManager::memory();
    let pool = Pool::new(manager)?;

    // Initialize schema on a connection from the pool
    {
        let conn = pool.get()?;
        initialize_schema(&conn)?;
    }

    Ok(pool)
}
