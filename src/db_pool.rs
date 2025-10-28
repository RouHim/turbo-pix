use log::info;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::ffi::sqlite3_auto_extension;
use sqlite_vec::sqlite3_vec_init;

use crate::db_schema::initialize_schema;

pub type DbPool = r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>;

// Pool sizing configuration
// Formula: (max_concurrent_photo_tasks() * 2) + API_REQUEST_BUFFER
// - *2 multiplier: Each task may need multiple connections during processing
// - API buffer: Reserve connections for concurrent API requests
const API_REQUEST_BUFFER: usize = 10;

/// Returns optimal number of concurrent photo processing tasks based on CPU cores
/// Formula: num_cores (for CPU-bound CLIP inference)
pub fn max_concurrent_photo_tasks() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4) // Fallback to 4 if detection fails
}

/// Calculate optimal database connection pool size
fn db_pool_size() -> usize {
    (max_concurrent_photo_tasks() * 2) + API_REQUEST_BUFFER
}

pub fn create_db_pool(database_path: &str) -> Result<DbPool, Box<dyn std::error::Error>> {
    if let Some(parent) = std::path::Path::new(database_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Register sqlite-vec extension for vector operations
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int,
        >(sqlite3_vec_init as *const ())));
    }

    let manager = SqliteConnectionManager::file(database_path);
    let pool_size = db_pool_size();
    info!(
        "Creating database pool: {} connections ({} concurrent tasks, {} API buffer)",
        pool_size,
        max_concurrent_photo_tasks(),
        API_REQUEST_BUFFER
    );
    let pool = Pool::builder()
        .max_size(pool_size as u32)
        .connection_timeout(std::time::Duration::from_secs(30))
        .build(manager)?;

    {
        let conn = pool.get()?;
        // Configure SQLite for optimal performance with large datasets
        // - cache_size: 128MB cache (default: ~2MB)
        // - mmap_size: 512MB memory-mapped I/O for faster reads
        // - wal_autocheckpoint: Checkpoint every 10k pages (less frequent checkpoints)
        // - analysis_limit: Faster ANALYZE queries
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -128000;
             PRAGMA mmap_size = 536870912;
             PRAGMA wal_autocheckpoint = 10000;
             PRAGMA analysis_limit = 1000;",
        )?;
        initialize_schema(&conn)?;
    }

    Ok(pool)
}

pub fn delete_orphaned_photos(
    pool: &DbPool,
    existing_paths: &[String],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    if existing_paths.is_empty() {
        let mut stmt = conn.prepare("SELECT file_path FROM photos")?;
        let deleted_paths: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        conn.execute("DELETE FROM photos", [])?;
        conn.execute("DELETE FROM media_semantic_vectors", [])?;
        conn.execute("DELETE FROM semantic_vector_path_mapping", [])?;
        conn.execute("DELETE FROM video_semantic_metadata", [])?;
        info!("Deleted all photos and semantic vectors from database (no files found)");
        return Ok(deleted_paths);
    }

    let placeholders = existing_paths
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");

    let select_sql = format!(
        "SELECT file_path FROM photos WHERE file_path NOT IN ({})",
        placeholders
    );

    let params: Vec<&dyn rusqlite::ToSql> = existing_paths
        .iter()
        .map(|p| p as &dyn rusqlite::ToSql)
        .collect();

    let mut stmt = conn.prepare(&select_sql)?;
    let deleted_paths: Vec<String> = stmt
        .query_map(params.as_slice(), |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;

    let delete_sql = format!(
        "DELETE FROM photos WHERE file_path NOT IN ({})",
        placeholders
    );

    let deleted_photos = conn.execute(&delete_sql, params.as_slice())?;

    let vector_cache_sql = format!(
        "DELETE FROM semantic_vector_path_mapping WHERE path NOT IN ({})",
        placeholders
    );
    let deleted_vectors = conn.execute(&vector_cache_sql, params.as_slice())?;

    let metadata_sql = format!(
        "DELETE FROM video_semantic_metadata WHERE path NOT IN ({})",
        placeholders
    );
    conn.execute(&metadata_sql, params.as_slice())?;

    conn.execute(
        "DELETE FROM media_semantic_vectors WHERE rowid NOT IN (SELECT id FROM semantic_vector_path_mapping)",
        [],
    )?;

    info!(
        "Deleted {} orphaned photos and {} orphaned semantic vectors from database",
        deleted_photos, deleted_vectors
    );

    Ok(deleted_paths)
}

pub fn vacuum_database(pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
    let conn = pool.get()?;
    conn.execute("VACUUM", [])?;
    info!("Database vacuum completed");
    Ok(())
}

#[cfg(test)]
pub fn create_in_memory_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    // Register sqlite-vec extension for vector operations
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int,
        >(sqlite3_vec_init as *const ())));
    }

    let manager = SqliteConnectionManager::memory();
    let pool = Pool::builder()
        .max_size(db_pool_size() as u32)
        .connection_timeout(std::time::Duration::from_secs(30))
        .build(manager)?;

    // Initialize schema on a connection from the pool
    {
        let conn = pool.get()?;
        initialize_schema(&conn)?;
    }

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::IntoBytes;

    #[test]
    fn test_delete_orphaned_photos_cleans_feature_vectors() {
        // Create test pool
        let pool = create_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();

        // Insert test data
        conn.execute(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (1, '/path/to/photo1.jpg')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (2, '/path/to/photo2.jpg')",
            [],
        )
        .unwrap();

        // Create dummy feature vectors
        let dummy_feature_vector = vec![0.0f32; 512];
        conn.execute(
            "INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (1, ?)",
            [dummy_feature_vector.as_slice().as_bytes()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (2, ?)",
            [dummy_feature_vector.as_slice().as_bytes()],
        )
        .unwrap();

        // Verify initial state
        let cache_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM semantic_vector_path_mapping",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cache_count, 2);

        let feature_vector_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM media_semantic_vectors", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(feature_vector_count, 2);

        drop(conn);

        // Delete orphaned photos (only keep photo1)
        let existing_paths = vec!["/path/to/photo1.jpg".to_string()];
        delete_orphaned_photos(&pool, &existing_paths).unwrap();

        // Verify cleanup
        let conn = pool.get().unwrap();
        let cache_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM semantic_vector_path_mapping",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cache_count, 1, "Should have 1 cached feature vector");

        let feature_vector_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM media_semantic_vectors", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(feature_vector_count, 1, "Should have 1 feature vector");

        let remaining_path: String = conn
            .query_row("SELECT path FROM semantic_vector_path_mapping", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining_path, "/path/to/photo1.jpg");
    }

    #[test]
    fn test_delete_all_photos_cleans_all_feature_vectors() {
        // Create test pool
        let pool = create_in_memory_pool().unwrap();
        let conn = pool.get().unwrap();

        // Insert test data
        conn.execute(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (1, '/path/to/photo1.jpg')",
            [],
        )
        .unwrap();

        let dummy_feature_vector = vec![0.0f32; 512];
        conn.execute(
            "INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (1, ?)",
            [dummy_feature_vector.as_slice().as_bytes()],
        )
        .unwrap();

        drop(conn);

        // Delete all photos (empty existing_paths)
        delete_orphaned_photos(&pool, &[]).unwrap();

        // Verify all cleaned up
        let conn = pool.get().unwrap();
        let cache_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM semantic_vector_path_mapping",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cache_count, 0);

        let feature_vector_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM media_semantic_vectors", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(feature_vector_count, 0);
    }
}
