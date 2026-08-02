use libsqlite3_sys::sqlite3_auto_extension;
use log::{info, warn};
use sqlite_vec::sqlite3_vec_init;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::str::FromStr;
use std::time::Duration;

pub type DbPool = sqlx::SqlitePool;

// Pool sizing configuration
// Formula: (max_concurrent_photo_tasks() * 2) + API_REQUEST_BUFFER
// - *2 multiplier: Each task may need multiple connections during processing
// - API buffer: Reserve connections for concurrent API requests
const API_REQUEST_BUFFER: usize = 2;

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

pub async fn create_db_pool(database_path: &str) -> Result<DbPool, Box<dyn std::error::Error>> {
    // Create parent directory
    if let Some(parent) = std::path::Path::new(database_path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Register sqlite-vec extension for vector operations
    // This must be done before creating any connections
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut libsqlite3_sys::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const libsqlite3_sys::sqlite3_api_routines,
            ) -> std::os::raw::c_int,
        >(sqlite3_vec_init as *const ())));
    }

    // Build connection options with PRAGMAs (extracted for clarity)
    fn build_connect_options(
        database_path: &str,
    ) -> Result<SqliteConnectOptions, Box<dyn std::error::Error>> {
        let base = SqliteConnectOptions::from_str(&format!("sqlite://{}", database_path))
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        Ok(base
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(30))
            .pragma("temp_store", "MEMORY")
            .pragma("cache_size", "-32000") // 32MB cache
            .pragma("mmap_size", "268435456") // 256MB memory-mapped I/O
            .pragma("wal_autocheckpoint", "10000")
            .pragma("analysis_limit", "1000"))
    }

    let connect_options = build_connect_options(database_path)?;

    // Calculate pool size
    let pool_size = db_pool_size();
    info!(
        "Creating database pool: {} connections ({} concurrent tasks, {} API buffer)",
        pool_size,
        max_concurrent_photo_tasks(),
        API_REQUEST_BUFFER
    );

    // Create pool
    let pool = SqlitePoolOptions::new()
        .max_connections(pool_size as u32)
        .min_connections(2) // Keep minimum connections alive
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(connect_options)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

pub async fn delete_orphaned_photos(
    pool: &DbPool,
    existing_paths: &[String],
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    if existing_paths.is_empty() {
        warn!("No files found on disk — skipping orphan cleanup to prevent accidental data loss");
        return Ok(Vec::new());
    }

    // Chunked temp-table approach: a single NOT IN with one bound parameter
    // per on-disk file would exceed SQLite's SQLITE_MAX_VARIABLE_NUMBER
    // (32766) for large libraries, making the statements fail to prepare and
    // silently killing nightly orphan cleanup forever.
    const CHUNK_SIZE: usize = 500;

    // All statements run on one held connection because the temp table is
    // per-connection. Autocommit (no explicit transaction): the temp-table
    // inserts never touch the main database, so no write lock is held while
    // they run and concurrent API traffic stays responsive.
    let mut conn = pool.acquire().await?;
    sqlx::query("CREATE TEMP TABLE scanned_paths (path TEXT PRIMARY KEY)")
        .execute(&mut *conn)
        .await?;
    for chunk in existing_paths.chunks(CHUNK_SIZE) {
        let rows = chunk.iter().map(|_| "(?)").collect::<Vec<_>>().join(",");
        let sql = format!("INSERT OR IGNORE INTO scanned_paths (path) VALUES {}", rows);
        let mut query = sqlx::query(&sql);
        for path in chunk {
            query = query.bind(path);
        }
        query.execute(&mut *conn).await?;
    }

    // Return (path, hash) pairs so callers can clear the hash-based
    // thumbnail cache for each deleted photo.
    let deleted_paths: Vec<(String, String)> = sqlx::query_as(
        "SELECT file_path, hash_sha256 FROM photos \
         WHERE file_path NOT IN (SELECT path FROM scanned_paths)",
    )
    .fetch_all(&mut *conn)
    .await?;

    // Delete orphaned photos
    let deleted_photos =
        sqlx::query("DELETE FROM photos WHERE file_path NOT IN (SELECT path FROM scanned_paths)")
            .execute(&mut *conn)
            .await?
            .rows_affected();

    // Delete orphaned vectors
    let deleted_vectors = sqlx::query(
        "DELETE FROM semantic_vector_path_mapping \
         WHERE path NOT IN (SELECT path FROM scanned_paths)",
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();

    // Delete orphaned video metadata (ignore rows affected)
    sqlx::query(
        "DELETE FROM video_semantic_metadata \
         WHERE path NOT IN (SELECT path FROM scanned_paths)",
    )
    .execute(&mut *conn)
    .await?;

    // Clean up orphaned vectors
    sqlx::query(
        "DELETE FROM media_semantic_vectors \
         WHERE rowid NOT IN (SELECT id FROM semantic_vector_path_mapping)",
    )
    .execute(&mut *conn)
    .await?;

    // Temp tables live on the pooled connection; drop it so a later call
    // (possibly on the same connection) can recreate it.
    sqlx::query("DROP TABLE scanned_paths")
        .execute(&mut *conn)
        .await?;

    info!(
        "Deleted {} orphaned photos and {} orphaned semantic vectors from database",
        deleted_photos, deleted_vectors
    );

    Ok(deleted_paths)
}

pub async fn vacuum_database(pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
    // The nightly vacuum runs while API handlers may still be writing; in WAL
    // mode VACUUM needs exclusive access and would otherwise block all DB
    // writes for its full duration. A short busy timeout makes it fail fast
    // (and be skipped for the night) instead of stalling requests.
    let mut conn = pool.acquire().await?;
    sqlx::query("PRAGMA busy_timeout = 1000")
        .execute(&mut *conn)
        .await?;
    sqlx::query("VACUUM").execute(&mut *conn).await?;
    info!("Database vacuum completed");
    Ok(())
}

#[cfg(test)]
pub async fn create_in_memory_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    // Register sqlite-vec extension for vector operations
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut libsqlite3_sys::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const libsqlite3_sys::sqlite3_api_routines,
            ) -> std::os::raw::c_int,
        >(sqlite3_vec_init as *const ())));
    }

    let options = SqliteConnectOptions::from_str("sqlite::memory:")?.create_if_missing(true);

    // CRITICAL: In-memory databases must use max_connections(1)
    // SQLite in-memory databases are connection-specific
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zerocopy::IntoBytes;

    #[tokio::test]
    async fn test_delete_orphaned_photos_cleans_feature_vectors() {
        // Create test pool
        let pool = create_in_memory_pool().await.unwrap();

        // Insert test data
        sqlx::query(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (1, '/path/to/photo1.jpg')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (2, '/path/to/photo2.jpg')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Create dummy feature vectors
        let dummy_feature_vector = vec![0.0f32; 512];
        let vector_bytes = dummy_feature_vector.as_slice().as_bytes();

        sqlx::query("INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (1, ?)")
            .bind(vector_bytes)
            .execute(&pool)
            .await
            .unwrap();

        sqlx::query("INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (2, ?)")
            .bind(vector_bytes)
            .execute(&pool)
            .await
            .unwrap();

        // Verify initial state
        let cache_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM semantic_vector_path_mapping")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cache_count, 2);

        let feature_vector_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM media_semantic_vectors")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(feature_vector_count, 2);

        // Delete orphaned photos (only keep photo1)
        let existing_paths = vec!["/path/to/photo1.jpg".to_string()];
        delete_orphaned_photos(&pool, &existing_paths)
            .await
            .unwrap();

        // Verify cleanup
        let cache_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM semantic_vector_path_mapping")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cache_count, 1, "Should have 1 cached feature vector");

        let feature_vector_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM media_semantic_vectors")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(feature_vector_count, 1, "Should have 1 feature vector");

        let remaining_path: String =
            sqlx::query_scalar("SELECT path FROM semantic_vector_path_mapping")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining_path, "/path/to/photo1.jpg");
    }

    #[tokio::test]
    async fn test_empty_paths_preserves_data() {
        let pool = create_in_memory_pool().await.unwrap();

        sqlx::query(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (1, '/path/to/photo1.jpg')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let dummy_feature_vector = vec![0.0f32; 512];
        let vector_bytes = dummy_feature_vector.as_slice().as_bytes();

        sqlx::query("INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (1, ?)")
            .bind(vector_bytes)
            .execute(&pool)
            .await
            .unwrap();

        let deleted = delete_orphaned_photos(&pool, &[]).await.unwrap();
        assert!(
            deleted.is_empty(),
            "Should not delete anything when no files found on disk"
        );

        let cache_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM semantic_vector_path_mapping")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cache_count, 1, "Data should be preserved");

        let feature_vector_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM media_semantic_vectors")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(feature_vector_count, 1, "Data should be preserved");
    }

    #[tokio::test]
    async fn test_delete_orphaned_photos_chunks_above_sqlite_variable_limit() {
        // GIVEN a library larger than SQLITE_MAX_VARIABLE_NUMBER (32766)
        // plus one DB row whose file has vanished
        let pool = create_in_memory_pool().await.unwrap();
        let orphan_path = "/path/to/orphan.jpg";
        sqlx::query(
            "INSERT INTO photos (hash_sha256, file_path, filename, file_size, file_modified) \
             VALUES (?, ?, 'orphan.jpg', 0, ?)",
        )
        .bind("0".repeat(64))
        .bind(orphan_path)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let existing_paths: Vec<String> = (0..33_000)
            .map(|i| format!("/path/to/existing/{}.jpg", i))
            .collect();

        // WHEN orphan cleanup runs with a single NOT IN over all of them
        let deleted = delete_orphaned_photos(&pool, &existing_paths)
            .await
            .unwrap();

        // THEN the orphan is found and removed (the old single-statement
        // placeholder list would fail to prepare at 33000 bound parameters)
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].0, orphan_path);
        assert_eq!(deleted[0].1, "0".repeat(64));
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
