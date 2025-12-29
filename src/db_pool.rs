use log::info;
use libsqlite3_sys::sqlite3_auto_extension;
use sqlite_vec::sqlite3_vec_init;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::str::FromStr;
use std::time::Duration;

pub type DbPool = sqlx::SqlitePool;

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

    // Build connection options with PRAGMAs
    let connect_options = SqliteConnectOptions::from_str(&format!("sqlite://{}", database_path))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(30))
        .pragma("temp_store", "MEMORY")
        .pragma("cache_size", "-128000")  // 128MB cache
        .pragma("mmap_size", "536870912")  // 512MB memory-mapped I/O
        .pragma("wal_autocheckpoint", "10000")
        .pragma("analysis_limit", "1000");

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
        .min_connections(2)  // Keep minimum connections alive
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(connect_options)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    Ok(pool)
}

pub async fn delete_orphaned_photos(
    pool: &DbPool,
    existing_paths: &[String],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if existing_paths.is_empty() {
        let deleted_paths: Vec<String> = sqlx::query_scalar("SELECT file_path FROM photos")
            .fetch_all(pool)
            .await?;

        sqlx::query("DELETE FROM photos")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM media_semantic_vectors")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM semantic_vector_path_mapping")
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM video_semantic_metadata")
            .execute(pool)
            .await?;

        info!("Deleted all photos and semantic vectors from database (no files found)");
        return Ok(deleted_paths);
    }

    // Build placeholders for IN clause
    let placeholders = existing_paths
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");

    let select_sql = format!(
        "SELECT file_path FROM photos WHERE file_path NOT IN ({})",
        placeholders
    );

    // Build query with dynamic parameters
    let mut select_query = sqlx::query_scalar::<_, String>(&select_sql);
    for path in existing_paths {
        select_query = select_query.bind(path);
    }
    let deleted_paths: Vec<String> = select_query.fetch_all(pool).await?;

    // Delete orphaned photos
    let delete_sql = format!(
        "DELETE FROM photos WHERE file_path NOT IN ({})",
        placeholders
    );
    let mut delete_query = sqlx::query(&delete_sql);
    for path in existing_paths {
        delete_query = delete_query.bind(path);
    }
    let deleted_photos = delete_query.execute(pool).await?.rows_affected();

    // Delete orphaned vectors
    let vector_cache_sql = format!(
        "DELETE FROM semantic_vector_path_mapping WHERE path NOT IN ({})",
        placeholders
    );
    let mut vector_query = sqlx::query(&vector_cache_sql);
    for path in existing_paths {
        vector_query = vector_query.bind(path);
    }
    let deleted_vectors = vector_query.execute(pool).await?.rows_affected();

    // Delete orphaned video metadata
    let metadata_sql = format!(
        "DELETE FROM video_semantic_metadata WHERE path NOT IN ({})",
        placeholders
    );
    let mut metadata_query = sqlx::query(&metadata_sql);
    for path in existing_paths {
        metadata_query = metadata_query.bind(path);
    }
    metadata_query.execute(pool).await?;

    // Clean up orphaned vectors
    sqlx::query(
        "DELETE FROM media_semantic_vectors WHERE rowid NOT IN (SELECT id FROM semantic_vector_path_mapping)"
    )
    .execute(pool)
    .await?;

    info!(
        "Deleted {} orphaned photos and {} orphaned semantic vectors from database",
        deleted_photos, deleted_vectors
    );

    Ok(deleted_paths)
}

pub async fn vacuum_database(pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query("VACUUM")
        .execute(pool)
        .await?;
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

    let options = SqliteConnectOptions::from_str("sqlite::memory:")?
        .create_if_missing(true);

    // CRITICAL: In-memory databases must use max_connections(1)
    // SQLite in-memory databases are connection-specific
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

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
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (1, '/path/to/photo1.jpg')"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (2, '/path/to/photo2.jpg')"
        )
        .execute(&pool)
        .await
        .unwrap();

        // Create dummy feature vectors
        let dummy_feature_vector = vec![0.0f32; 512];
        let vector_bytes = dummy_feature_vector.as_slice().as_bytes();

        sqlx::query(
            "INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (1, ?)"
        )
        .bind(vector_bytes)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (2, ?)"
        )
        .bind(vector_bytes)
        .execute(&pool)
        .await
        .unwrap();

        // Verify initial state
        let cache_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM semantic_vector_path_mapping"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cache_count, 2);

        let feature_vector_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM media_semantic_vectors"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(feature_vector_count, 2);

        // Delete orphaned photos (only keep photo1)
        let existing_paths = vec!["/path/to/photo1.jpg".to_string()];
        delete_orphaned_photos(&pool, &existing_paths).await.unwrap();

        // Verify cleanup
        let cache_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM semantic_vector_path_mapping"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cache_count, 1, "Should have 1 cached feature vector");

        let feature_vector_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM media_semantic_vectors"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(feature_vector_count, 1, "Should have 1 feature vector");

        let remaining_path: String = sqlx::query_scalar(
            "SELECT path FROM semantic_vector_path_mapping"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(remaining_path, "/path/to/photo1.jpg");
    }

    #[tokio::test]
    async fn test_delete_all_photos_cleans_all_feature_vectors() {
        // Create test pool
        let pool = create_in_memory_pool().await.unwrap();

        // Insert test data
        sqlx::query(
            "INSERT INTO semantic_vector_path_mapping (id, path) VALUES (1, '/path/to/photo1.jpg')"
        )
        .execute(&pool)
        .await
        .unwrap();

        let dummy_feature_vector = vec![0.0f32; 512];
        let vector_bytes = dummy_feature_vector.as_slice().as_bytes();

        sqlx::query(
            "INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (1, ?)"
        )
        .bind(vector_bytes)
        .execute(&pool)
        .await
        .unwrap();

        // Delete all photos (empty existing_paths)
        delete_orphaned_photos(&pool, &[]).await.unwrap();

        // Verify all cleaned up
        let cache_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM semantic_vector_path_mapping"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cache_count, 0);

        let feature_vector_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM media_semantic_vectors"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(feature_vector_count, 0);
    }
}
