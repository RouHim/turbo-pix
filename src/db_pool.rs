use chrono::{DateTime, Utc};
use log::info;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use crate::db_schema::initialize_schema;

pub type DbPool = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;

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
