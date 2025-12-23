use clokwerk::{Job, Scheduler, TimeUnits};
use log::{error, info, warn};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::Mutex;

use crate::cache_manager::CacheManager;
use crate::collage_generator;
use crate::db::DbPool;
use crate::indexer::PhotoProcessor;
use crate::semantic_search::SemanticSearchEngine;

/// Batch size for database writes (I/O-bound operations)
/// Larger batches (250) reduce transaction overhead and improve throughput.
/// Higher than SEMANTIC_BATCH_SIZE since database writes are I/O-bound
/// and benefit from larger transactions to amortize COMMIT costs.
const DB_WRITE_BATCH_SIZE: usize = 250;

/// Indexing status shared across threads
#[derive(Clone)]
pub struct IndexingStatus {
    pub is_indexing: Arc<AtomicBool>,
    pub current_phase: Arc<Mutex<String>>,
    pub photos_total: Arc<AtomicU64>,
    pub photos_processed: Arc<AtomicU64>,
    pub photos_semantic_indexed: Arc<AtomicU64>,
    pub started_at: Arc<Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl IndexingStatus {
    pub fn new() -> Self {
        Self {
            is_indexing: Arc::new(AtomicBool::new(false)),
            current_phase: Arc::new(Mutex::new(String::from("idle"))),
            photos_total: Arc::new(AtomicU64::new(0)),
            photos_processed: Arc::new(AtomicU64::new(0)),
            photos_semantic_indexed: Arc::new(AtomicU64::new(0)),
            started_at: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start_indexing(&self) {
        self.is_indexing.store(true, Ordering::SeqCst);
        *self.started_at.lock().await = Some(chrono::Utc::now());
        self.photos_total.store(0, Ordering::SeqCst);
        self.photos_processed.store(0, Ordering::SeqCst);
        self.photos_semantic_indexed.store(0, Ordering::SeqCst);
    }

    pub async fn stop_indexing(&self) {
        self.is_indexing.store(false, Ordering::SeqCst);
        *self.current_phase.lock().await = String::from("idle");
    }

    pub async fn set_phase(&self, phase: &str) {
        *self.current_phase.lock().await = String::from(phase);
    }
}

#[derive(Clone)]
pub struct PhotoScheduler {
    photo_paths: Vec<PathBuf>,
    db_pool: DbPool,
    cache_manager: CacheManager,
    semantic_search: Arc<SemanticSearchEngine>,
    data_path: PathBuf,
    locale: String,
    rescan_lock: Arc<Mutex<()>>,
    pub status: IndexingStatus,
}

impl PhotoScheduler {
    pub fn new(
        photo_paths: Vec<PathBuf>,
        db_pool: DbPool,
        cache_manager: CacheManager,
        semantic_search: Arc<SemanticSearchEngine>,
        data_path: PathBuf,
        locale: String,
    ) -> Self {
        let mut photo_paths = photo_paths;
        let collages_path = data_path.join("collages").join("accepted");

        if !photo_paths.contains(&collages_path) {
            if let Err(e) = std::fs::create_dir_all(&collages_path) {
                warn!(
                    "Failed to create collages directory {}: {}",
                    collages_path.display(),
                    e
                );
            }
            photo_paths.push(collages_path);
        }

        Self {
            photo_paths,
            db_pool,
            cache_manager,
            semantic_search,
            data_path,
            locale,
            rescan_lock: Arc::new(Mutex::new(())),
            status: IndexingStatus::new(),
        }
    }

    /// Helper: Batch write photos to database with transaction support
    ///
    /// Writes photos in batches to minimize database lock duration.
    /// Uses rusqlite's transaction API with IMMEDIATE mode for better concurrency.
    ///
    /// Returns (successful_count, error_count)
    fn batch_write_photos(
        db_pool: &DbPool,
        photos: Vec<crate::db::Photo>,
        status: &IndexingStatus,
    ) -> (usize, usize) {
        let mut indexed_count = 0;
        let mut error_count = 0;
        let total_count = photos.len();

        // Update total count
        status
            .photos_total
            .store(total_count as u64, Ordering::SeqCst);

        for (batch_idx, batch) in photos.chunks(DB_WRITE_BATCH_SIZE).enumerate() {
            match db_pool.get() {
                Ok(mut conn) => {
                    // Use rusqlite's transaction API with IMMEDIATE mode
                    let tx = match conn
                        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    {
                        Ok(tx) => tx,
                        Err(e) => {
                            error!("Failed to begin transaction: {}", e);
                            error_count += batch.len();
                            continue;
                        }
                    };

                    let mut batch_success = 0;

                    for photo in batch {
                        match photo.create_or_update_with_connection(&tx) {
                            Ok(_) => batch_success += 1,
                            Err(e) => {
                                error!("Failed to save photo {}: {}", photo.file_path, e);
                                error_count += 1;
                            }
                        }
                    }

                    // Commit transaction (partial success is ok - errors already counted)
                    match tx.commit() {
                        Ok(_) => {
                            indexed_count += batch_success;
                            // Update progress counter
                            status
                                .photos_processed
                                .store(indexed_count as u64, Ordering::SeqCst);
                            info!(
                                "Batch {}/{} committed: {} photos saved{}",
                                batch_idx + 1,
                                total_count.div_ceil(DB_WRITE_BATCH_SIZE),
                                batch_success,
                                if error_count > 0 {
                                    format!(", {} errors so far", error_count)
                                } else {
                                    String::new()
                                }
                            );
                        }
                        Err(e) => {
                            error!("Failed to commit batch: {}", e);
                            error_count += batch_success; // Count successful photos as errors if commit fails
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to get database connection: {}", e);
                    error_count += batch.len();
                }
            }

            // Progress logging every 5 batches
            if (batch_idx + 1) % 5 == 0 {
                info!(
                    "Progress: {}/{} photos processed ({:.1}%), {} errors",
                    indexed_count + error_count,
                    total_count,
                    ((indexed_count + error_count) as f64 / total_count as f64) * 100.0,
                    error_count
                );
            }
        }

        (indexed_count, error_count)
    }

    pub fn start(&self) -> JoinHandle<()> {
        let mut scheduler = Scheduler::new();

        let photo_paths = self.photo_paths.clone();
        let db_pool = self.db_pool.clone();
        let cache_manager = self.cache_manager.clone();
        let semantic_search = self.semantic_search.clone();
        let data_path = self.data_path.clone();
        let rescan_lock = self.rescan_lock.clone();
        let status = self.status.clone();
        let locale = self.locale.clone();

        // Full rescan and cleanup at midnight
        scheduler.every(1.day()).at("00:00").run(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                // Try to acquire lock - if another rescan is running, skip this one
                let lock = match rescan_lock.try_lock() {
                    Ok(lock) => lock,
                    Err(_) => {
                        warn!("Skipping scheduled rescan - another rescan is already in progress");
                        return;
                    }
                };

                info!("Starting scheduled photo rescan and cleanup (two-phase indexing)");

                // Mark indexing as started
                status.start_indexing().await;
                status.set_phase("metadata").await;

                let processor = PhotoProcessor::new(photo_paths.clone(), semantic_search.clone());

                // Phase 1: Fast metadata-only scan (skip semantic vectors)
                info!("Phase 1: Fast metadata scan (skipping semantic vectors)");
                match processor
                    .full_rescan_and_cleanup(&db_pool, &cache_manager)
                    .await
                {
                    Ok(processed_photos) => {
                        // Convert ProcessedPhoto to Photo
                        let photos: Vec<crate::db::Photo> =
                            processed_photos.into_iter().map(|p| p.into()).collect();

                        // Batch write photos to database
                        let (indexed_count, error_count) =
                            Self::batch_write_photos(&db_pool, photos, &status);

                        info!(
                            "Phase 1 completed: {} photos indexed, {} errors",
                            indexed_count, error_count
                        );

                        // Phase 2: Batch compute semantic vectors
                        info!("Phase 2: Computing semantic vectors in batches");
                        status.set_phase("semantic_vectors").await;
                        match processor.batch_compute_semantic_vectors(&db_pool).await {
                            Ok((success, errors)) => {
                                status
                                    .photos_semantic_indexed
                                    .store(success as u64, Ordering::SeqCst);
                                info!(
                                    "Phase 2 completed: {} semantic vectors computed, {} errors",
                                    success, errors
                                )
                            }
                            Err(e) => error!("Phase 2 failed: {}", e),
                        }

                        // Phase 3: Generate collages
                        info!("Phase 3: Generating collages");
                        status.set_phase("collages").await;
                        match collage_generator::generate_collages(
                            &db_pool,
                            &data_path,
                            locale.as_str(),
                        )
                        .await
                        {
                            Ok(count) => info!("Phase 3 completed: {} collages generated", count),
                            Err(e) => error!("Phase 3 (collage generation) failed: {}", e),
                        }
                    }
                    Err(e) => error!("Phase 1 (metadata scan) failed: {}", e),
                }

                // Mark indexing as completed
                status.stop_indexing().await;
                drop(lock);
            });
        });

        // Database vacuum at 00:05 (after rescan completes)
        let db_pool_vacuum = self.db_pool.clone();
        scheduler.every(1.day()).at("00:05").run(move || {
            info!("Starting scheduled database vacuum");

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match crate::db::vacuum_database(&db_pool_vacuum) {
                    Ok(_) => info!("Database vacuum completed successfully"),
                    Err(e) => error!("Database vacuum failed: {}", e),
                }
            });
        });

        let handle = thread::spawn(move || loop {
            scheduler.run_pending();
            thread::sleep(Duration::from_secs(60));
        });

        info!("Photo scheduler started - Full rescan at 00:00, vacuum at 00:05");
        handle
    }

    pub async fn run_startup_rescan(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Try to acquire lock - if another rescan is running, skip this one
        let lock = match self.rescan_lock.try_lock() {
            Ok(lock) => lock,
            Err(_) => {
                warn!("Skipping startup rescan - another rescan is already in progress");
                return Ok(());
            }
        };

        info!("Starting startup photo rescan and cleanup (two-phase indexing)");

        // Mark indexing as started
        self.status.start_indexing().await;
        self.status.set_phase("metadata").await;

        let processor = PhotoProcessor::new(self.photo_paths.clone(), self.semantic_search.clone());

        // Phase 1: Fast metadata-only scan (skip semantic vectors)
        info!("Phase 1: Fast metadata scan (skipping semantic vectors)");
        let processed_photos = processor
            .full_rescan_and_cleanup(&self.db_pool, &self.cache_manager)
            .await?;

        // Convert ProcessedPhoto to Photo
        let photos: Vec<crate::db::Photo> =
            processed_photos.into_iter().map(|p| p.into()).collect();

        // Batch write photos to database (same as scheduled rescan)
        let (indexed_count, error_count) =
            Self::batch_write_photos(&self.db_pool, photos, &self.status);

        info!(
            "Phase 1 completed: {} photos indexed, {} errors",
            indexed_count, error_count
        );

        // Phase 2: Batch compute semantic vectors
        info!("Phase 2: Computing semantic vectors in batches");
        self.status.set_phase("semantic_vectors").await;
        match processor
            .batch_compute_semantic_vectors(&self.db_pool)
            .await
        {
            Ok((success, errors)) => {
                self.status
                    .photos_semantic_indexed
                    .store(success as u64, Ordering::SeqCst);
                info!(
                    "Phase 2 completed: {} semantic vectors computed, {} errors",
                    success, errors
                )
            }
            Err(e) => error!("Phase 2 failed: {}", e),
        }

        // Phase 3: Generate collages
        info!("Phase 3: Generating collages");
        self.status.set_phase("collages").await;
        match collage_generator::generate_collages(&self.db_pool, &self.data_path, &self.locale)
            .await
        {
            Ok(count) => info!("Phase 3 completed: {} collages generated", count),
            Err(e) => error!("Phase 3 (collage generation) failed: {}", e),
        }

        info!(
            "Startup rescan completed (three-phase): {} photos indexed, {} errors",
            indexed_count, error_count
        );

        // Mark indexing as completed
        self.status.stop_indexing().await;
        drop(lock);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;
    use tokio::time::{sleep, Duration as TokioDuration};

    use crate::cache_manager::CacheManager;
    use crate::db::{create_test_db_pool, DbPool, Photo};

    struct TestEnvironment {
        temp_dir: TempDir,
        db_pool: DbPool,
        cache_manager: CacheManager,
        scheduler: PhotoScheduler,
    }

    impl TestEnvironment {
        async fn new() -> Self {
            let temp_dir = TempDir::new().unwrap();
            let db_pool = create_test_db_pool().unwrap();
            let cache_manager = CacheManager::new(temp_dir.path().join("cache").to_path_buf());
            let semantic_search =
                Arc::new(SemanticSearchEngine::new(db_pool.clone(), "./data").unwrap());

            let photo_paths = vec![temp_dir.path().to_path_buf()];
            let data_path = temp_dir.path().to_path_buf();

            let scheduler = PhotoScheduler::new(
                photo_paths,
                db_pool.clone(),
                cache_manager.clone(),
                semantic_search,
                data_path,
                "en".to_string(),
            );

            Self {
                temp_dir,
                db_pool,
                cache_manager,
                scheduler,
            }
        }

        fn create_test_image(&self, filename: &str, content: &[u8]) -> PathBuf {
            let file_path = self.temp_dir.path().join(filename);
            let mut file = File::create(&file_path).unwrap();
            file.write_all(content).unwrap();
            file_path
        }

        async fn add_photo_to_db(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
            use chrono::Utc;
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            // Generate unique hash based on path (64 chars for SHA256 constraint)
            let mut hasher = DefaultHasher::new();
            path.hash(&mut hasher);
            let unique_hash = format!("{:016x}{}", hasher.finish(), "0".repeat(48));

            let now = Utc::now();
            let photo = Photo {
                hash_sha256: unique_hash,
                file_path: path.to_string(),
                filename: path.split('/').next_back().unwrap_or(path).to_string(),
                file_size: 1024,
                mime_type: Some("image/jpeg".to_string()),
                taken_at: None,
                width: Some(800),
                height: Some(600),
                orientation: Some(1),
                duration: None,
                thumbnail_path: None,
                has_thumbnail: Some(false),
                blurhash: None,
                is_favorite: Some(false),
                semantic_vector_indexed: Some(false),
                metadata: serde_json::json!({}),
                date_modified: now,
                date_indexed: Some(now),
                created_at: now,
                updated_at: now,
            };

            photo.create_or_update(&self.db_pool)?;
            Ok(())
        }

        async fn get_all_photo_paths(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
            crate::db::get_all_photo_paths(&self.db_pool)
        }
    }

    #[tokio::test]
    async fn test_photo_scheduler_new() {
        let env = TestEnvironment::new().await;
        assert!(!env.scheduler.photo_paths.is_empty());
    }

    #[tokio::test]
    async fn test_startup_rescan_empty_directory() {
        let env = TestEnvironment::new().await;

        let result = env.scheduler.run_startup_rescan().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_startup_rescan_with_new_files() {
        let env = TestEnvironment::new().await;

        // Create test images
        env.create_test_image("test1.jpg", b"fake jpeg content 1");
        env.create_test_image("test2.png", b"fake png content 2");

        let result = env.scheduler.run_startup_rescan().await;
        assert!(result.is_ok());

        // Verify files were processed
        let db_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(db_paths.len(), 2);
    }

    #[tokio::test]
    async fn test_startup_rescan_skips_unchanged_files() {
        let env = TestEnvironment::new().await;

        // Create test image and add to database
        let image_path = env.create_test_image("test.jpg", b"test content");
        env.add_photo_to_db(&image_path.to_string_lossy())
            .await
            .unwrap();

        // Run startup rescan
        let result = env.scheduler.run_startup_rescan().await;
        assert!(result.is_ok());

        // Should still have 1 photo in database
        let db_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(db_paths.len(), 1);
    }

    #[tokio::test]
    async fn test_orphaned_file_cleanup() {
        let env = TestEnvironment::new().await;

        // Add a photo to database but don't create the file
        let fake_path = env.temp_dir.path().join("nonexistent.jpg");
        env.add_photo_to_db(&fake_path.to_string_lossy())
            .await
            .unwrap();

        // Create an actual image file
        env.create_test_image("real.jpg", b"real image content");

        // Verify we have 1 photo in database initially
        let initial_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(initial_paths.len(), 1);

        // Run startup rescan (should clean up orphaned entry)
        let result = env.scheduler.run_startup_rescan().await;
        assert!(result.is_ok());

        // Should now have only the real file
        let final_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(final_paths.len(), 1);
        assert!(final_paths[0].contains("real.jpg"));
    }

    #[tokio::test]
    async fn test_rescan_detects_file_modifications() {
        let env = TestEnvironment::new().await;

        // Create and index initial file
        let image_path = env.create_test_image("test.jpg", b"original content");
        env.scheduler.run_startup_rescan().await.unwrap();

        // Verify file is in database
        let initial_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(initial_paths.len(), 1);

        // Wait a bit to ensure different timestamp
        sleep(TokioDuration::from_millis(10)).await;

        // Modify the file
        let mut file = File::create(&image_path).unwrap();
        file.write_all(b"modified content").unwrap();

        // Run rescan again
        let result = env.scheduler.run_startup_rescan().await;
        assert!(result.is_ok());

        // Should still have 1 photo but it should be updated
        let final_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(final_paths.len(), 1);
    }

    #[tokio::test]
    async fn test_database_vacuum_operations() {
        let env = TestEnvironment::new().await;

        // Add some test data
        env.create_test_image("test1.jpg", b"content 1");
        env.create_test_image("test2.jpg", b"content 2");
        env.scheduler.run_startup_rescan().await.unwrap();

        // Test vacuum operation (should not fail)
        let vacuum_result = crate::db::vacuum_database(&env.db_pool);
        assert!(vacuum_result.is_ok());
    }

    #[tokio::test]
    async fn test_cache_cleanup_operations() {
        let env = TestEnvironment::new().await;

        // Test cache cleanup
        let cleanup_result = env.cache_manager.clear_all().await;
        assert!(cleanup_result.is_ok());
    }

    #[tokio::test]
    async fn test_full_rescan_and_cleanup_comprehensive() {
        let env = TestEnvironment::new().await;

        // Setup complex scenario:
        // 1. Create some files
        let image1 = env.create_test_image("existing1.jpg", b"existing content 1");
        let _image2 = env.create_test_image("existing2.jpg", b"existing content 2");

        // 2. Add one to database, leave one new
        env.add_photo_to_db(&image1.to_string_lossy())
            .await
            .unwrap();

        // 3. Add orphaned entry to database
        let orphaned_path = env.temp_dir.path().join("deleted.jpg");
        env.add_photo_to_db(&orphaned_path.to_string_lossy())
            .await
            .unwrap();

        // Verify initial state
        let initial_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(initial_paths.len(), 2); // existing1.jpg + deleted.jpg

        // Run full rescan
        let result = env.scheduler.run_startup_rescan().await;
        assert!(result.is_ok());

        // Verify final state
        let final_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(final_paths.len(), 2); // existing1.jpg + existing2.jpg

        let final_filenames: HashSet<String> = final_paths
            .iter()
            .map(|p| p.split('/').next_back().unwrap_or(p).to_string())
            .collect();

        assert!(final_filenames.contains("existing1.jpg"));
        assert!(final_filenames.contains("existing2.jpg"));
        assert!(!final_filenames.contains("deleted.jpg"));
    }

    #[tokio::test]
    async fn test_scheduler_error_handling() {
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_test_db_pool().unwrap();
        let cache_manager = CacheManager::new(temp_dir.path().join("cache").to_path_buf());
        let semantic_search =
            Arc::new(SemanticSearchEngine::new(db_pool.clone(), "./data").unwrap());
        let data_path = temp_dir.path().to_path_buf();

        // Create scheduler with non-existent path
        let invalid_paths = vec![PathBuf::from("/nonexistent/path")];
        let scheduler = PhotoScheduler::new(
            invalid_paths,
            db_pool,
            cache_manager,
            semantic_search,
            data_path,
            "en".to_string(),
        );

        // Should handle errors gracefully
        let result = scheduler.run_startup_rescan().await;
        assert!(result.is_ok()); // Should not panic, just log errors
    }

    #[tokio::test]
    async fn test_concurrent_rescan_operations() {
        let env = TestEnvironment::new().await;

        // Create test files
        env.create_test_image("concurrent1.jpg", b"content 1");
        env.create_test_image("concurrent2.jpg", b"content 2");

        // Run multiple rescans concurrently
        let rescan1 = env.scheduler.run_startup_rescan();
        let rescan2 = env.scheduler.run_startup_rescan();

        let (result1, result2) = tokio::join!(rescan1, rescan2);

        // Both should complete successfully
        assert!(result1.is_ok());
        assert!(result2.is_ok());

        // Should have processed files correctly
        let final_paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(final_paths.len(), 2);
    }

    #[tokio::test]
    async fn test_phase2_batch_semantic_vectors() {
        let env = TestEnvironment::new().await;

        // Create test images
        env.create_test_image("semantic1.jpg", b"test image 1");
        env.create_test_image("semantic2.jpg", b"test image 2");
        env.create_test_image("semantic3.jpg", b"test image 3");

        // Add photos to database (simulating Phase 1)
        env.scheduler.run_startup_rescan().await.unwrap();

        // Verify photos were indexed
        let paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(paths.len(), 3);

        // Phase 2 should complete without errors
        // (Actual semantic vector computation may fail due to test data not being real images,
        //  but the batch processing logic should work)
        let processor = PhotoProcessor::new(
            env.scheduler.photo_paths.clone(),
            env.scheduler.semantic_search.clone(),
        );
        let result = processor.batch_compute_semantic_vectors(&env.db_pool).await;

        // Should complete - returns (success_count, error_count)
        assert!(result.is_ok());
        let (success, errors) = result.unwrap();
        assert_eq!(
            success + errors,
            3,
            "Should process exactly 3 photos, got {} successes and {} errors",
            success,
            errors
        );
    }

    #[tokio::test]
    async fn test_batch_write_consistency() {
        let env = TestEnvironment::new().await;

        // Create multiple test files to trigger batching
        for i in 0..10 {
            env.create_test_image(
                &format!("batch_test_{}.jpg", i),
                format!("content {}", i).as_bytes(),
            );
        }

        // Run startup rescan (uses batched writes)
        let result = env.scheduler.run_startup_rescan().await;
        assert!(result.is_ok());

        // Verify all files were indexed
        let paths = env.get_all_photo_paths().await.unwrap();
        assert_eq!(paths.len(), 10);

        // Run again to test idempotency
        let result2 = env.scheduler.run_startup_rescan().await;
        assert!(result2.is_ok());

        // Should still have 10 photos (no duplicates)
        let paths2 = env.get_all_photo_paths().await.unwrap();
        assert_eq!(paths2.len(), 10);
    }
}
