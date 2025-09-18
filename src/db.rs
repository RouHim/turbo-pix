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
    id INTEGER PRIMARY KEY AUTOINCREMENT,
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
    hash_md5 TEXT,
    hash_sha256 TEXT,
    thumbnail_path TEXT,
    has_thumbnail BOOLEAN,
    country TEXT,
    keywords TEXT,
    faces_detected TEXT,
    objects_detected TEXT,
    colors TEXT,
    created_at DATETIME,
    updated_at DATETIME
);
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
];

// Connection pool functions
pub fn create_db_pool(database_path: &str) -> Result<DbPool, Box<dyn std::error::Error>> {
    let manager = SqliteConnectionManager::file(database_path);
    let pool = Pool::new(manager)?;

    // Initialize schema on a connection from the pool
    {
        let conn = pool.get()?;
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
    pub id: i64,
    pub path: String,
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
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub location_name: Option<String>,
    pub hash_md5: Option<String>,
    pub hash_sha256: Option<String>,
    pub thumbnail_path: Option<String>,
    pub has_thumbnail: Option<bool>,
    pub country: Option<String>,
    pub keywords: Option<String>,
    pub faces_detected: Option<String>,
    pub objects_detected: Option<String>,
    pub colors: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Photo {
    pub fn from_row(row: &Row) -> SqlResult<Self> {
        Ok(Photo {
            id: row.get(0)?,
            path: row.get(1)?,
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
            gps_latitude: row.get(24)?,
            gps_longitude: row.get(25)?,
            location_name: row.get(26)?,
            hash_md5: row.get(27)?,
            hash_sha256: row.get(28)?,
            thumbnail_path: row.get(29)?,
            has_thumbnail: row.get(30)?,
            country: row.get(31)?,
            keywords: row.get(32)?,
            faces_detected: row.get(33)?,
            objects_detected: row.get(34)?,
            colors: row.get(35)?,
            created_at: {
                let datetime_str = row.get::<_, String>(36)?;
                if datetime_str.contains('T') {
                    DateTime::parse_from_rfc3339(&datetime_str)
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                36,
                                "created_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .with_timezone(&Utc)
                } else {
                    NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                36,
                                "created_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .and_utc()
                }
            },
            updated_at: {
                let datetime_str = row.get::<_, String>(37)?;
                if datetime_str.contains('T') {
                    DateTime::parse_from_rfc3339(&datetime_str)
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                37,
                                "updated_at".to_string(),
                                rusqlite::types::Type::Text,
                            )
                        })?
                        .with_timezone(&Utc)
                } else {
                    NaiveDateTime::parse_from_str(&datetime_str, "%Y-%m-%d %H:%M:%S")
                        .map_err(|_| {
                            rusqlite::Error::InvalidColumnType(
                                37,
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
                hash_md5 = ?, hash_sha256 = ?, thumbnail_path = ?, has_thumbnail = ?,
                country = ?, keywords = ?, faces_detected = ?, objects_detected = ?, colors = ?,
                updated_at = ?
             WHERE id = ?",
            rusqlite::params![
                self.path,
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
                self.gps_latitude,
                self.gps_longitude,
                self.location_name,
                self.hash_md5,
                self.hash_sha256,
                self.thumbnail_path,
                self.has_thumbnail,
                self.country,
                self.keywords,
                self.faces_detected,
                self.objects_detected,
                self.colors,
                Utc::now().to_rfc3339(),
                self.id
            ],
        )?;
        Ok(())
    }

    pub fn update_thumbnail_status(
        &self,
        pool: &DbPool,
        has_thumbnail: bool,
        thumbnail_path: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE photos SET has_thumbnail = ?, thumbnail_path = ? WHERE id = ?",
            rusqlite::params![has_thumbnail, thumbnail_path, self.id],
        )?;
        Ok(())
    }

    pub fn create_or_update(&self, pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Try to find existing photo by path
        let existing = conn.query_row(
            "SELECT id FROM photos WHERE file_path = ?",
            [&self.path],
            |row| row.get::<_, i64>(0),
        );

        if existing.is_ok() {
            // Update existing photo
            self.update(pool)
        } else {
            // Create new photo
            self.create(pool)?;
            Ok(())
        }
    }

    pub fn list_all(pool: &DbPool, limit: i64) -> Result<Vec<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare("SELECT * FROM photos ORDER BY taken_at DESC LIMIT ?")?;
        let photo_iter = stmt.query_map([limit], Photo::from_row)?;

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
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Get total count
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))?;

        // Get paginated results
        let mut stmt =
            conn.prepare("SELECT * FROM photos ORDER BY taken_at DESC LIMIT ? OFFSET ?")?;
        let photo_iter = stmt.query_map([limit, offset], Photo::from_row)?;

        let mut photos = Vec::new();
        for photo in photo_iter {
            photos.push(photo?);
        }
        Ok((photos, total))
    }

    pub fn find_by_id(pool: &DbPool, id: i64) -> Result<Option<Photo>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare("SELECT * FROM photos WHERE id = ?")?;

        match stmt.query_row([id], Photo::from_row) {
            Ok(photo) => Ok(Some(photo)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    pub fn create(&self, pool: &DbPool) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        let sql = r#"
            INSERT INTO photos (
                file_path, filename, file_size, taken_at, camera_make, camera_model,
                iso, aperture, shutter_speed, focal_length, width, height, orientation,
                flash_used, latitude, longitude, country, keywords,
                faces_detected, objects_detected, colors, file_modified, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
            )
        "#;

        conn.execute(
            sql,
            params![
                self.path,
                self.filename,
                self.file_size,
                self.taken_at.map(|dt| dt.to_rfc3339()),
                self.camera_make,
                self.camera_model,
                self.iso,
                self.aperture,
                self.shutter_speed,
                self.focal_length,
                self.width,
                self.height,
                self.orientation,
                self.flash_used,
                self.gps_latitude,
                self.gps_longitude,
                self.country,
                self.keywords,
                self.faces_detected,
                self.objects_detected,
                self.colors,
                self.date_modified.to_rfc3339(),
                Utc::now().to_rfc3339(),
                Utc::now().to_rfc3339(),
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    pub fn delete(pool: &DbPool, id: i64) -> Result<bool, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let deleted_rows = conn.execute("DELETE FROM photos WHERE id = ?", [id])?;
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
    ) -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>> {
        let conn = pool.get()?;

        // Build the WHERE clause (reusable for both count and data queries)
        let mut where_clause = String::from(" WHERE 1=1");
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref q) = query.q {
            where_clause.push_str(" AND (filename LIKE ? OR keywords LIKE ? OR camera_make LIKE ? OR camera_model LIKE ?)");
            let pattern = format!("%{}%", q);
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern.clone()));
            params.push(Box::new(pattern));
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
        let data_sql = format!(
            "SELECT * FROM photos{} ORDER BY taken_at DESC LIMIT ? OFFSET ?",
            where_clause
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
