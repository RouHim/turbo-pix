use clokwerk::{Job, Scheduler, TimeUnits};
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use log::{error, info};

use crate::cache::CacheManager;
use crate::db::DbPool;
use crate::indexer::PhotoProcessor;

#[derive(Clone)]
pub struct PhotoScheduler {
    photo_paths: Vec<PathBuf>,
    db_pool: DbPool,
    cache_manager: CacheManager,
}

impl PhotoScheduler {
    pub fn new(photo_paths: Vec<PathBuf>, db_pool: DbPool, cache_manager: CacheManager) -> Self {
        Self {
            photo_paths,
            db_pool,
            cache_manager,
        }
    }

    pub fn start(&self) -> JoinHandle<()> {
        let mut scheduler = Scheduler::new();

        let photo_paths = self.photo_paths.clone();
        let db_pool = self.db_pool.clone();
        let cache_manager = self.cache_manager.clone();

        // Full rescan and cleanup at midnight
        scheduler.every(1.day()).at("00:00").run(move || {
            info!("Starting scheduled photo rescan and cleanup");

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let processor = PhotoProcessor::new(photo_paths.clone());

                match processor
                    .full_rescan_and_cleanup(&db_pool, &cache_manager)
                    .await
                {
                    Ok(processed_photos) => {
                        let mut indexed_count = 0;
                        let mut error_count = 0;

                        for processed_photo in processed_photos {
                            let photo: crate::db::Photo = processed_photo.into();
                            match photo.create_or_update(&db_pool) {
                                Ok(_) => indexed_count += 1,
                                Err(e) => {
                                    error!("Failed to save photo to database: {}", e);
                                    error_count += 1;
                                }
                            }
                        }

                        info!(
                            "Scheduled rescan completed: {} photos processed, {} errors",
                            indexed_count, error_count
                        );
                    }
                    Err(e) => error!("Scheduled rescan failed: {}", e),
                }
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
        info!("Starting startup photo rescan and cleanup");

        let processor = PhotoProcessor::new(self.photo_paths.clone());

        let processed_photos = processor
            .full_rescan_and_cleanup(&self.db_pool, &self.cache_manager)
            .await?;

        let mut indexed_count = 0;
        let mut error_count = 0;

        for processed_photo in processed_photos {
            let photo: crate::db::Photo = processed_photo.into();
            match photo.create_or_update(&self.db_pool) {
                Ok(_) => indexed_count += 1,
                Err(e) => {
                    error!("Failed to save photo to database: {}", e);
                    error_count += 1;
                }
            }
        }

        info!(
            "Startup rescan completed: {} photos processed, {} errors",
            indexed_count, error_count
        );

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

    use crate::cache::CacheManager;
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

            let photo_paths = vec![temp_dir.path().to_path_buf()];
            let scheduler =
                PhotoScheduler::new(photo_paths, db_pool.clone(), cache_manager.clone());

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
                filename: path.split('/').last().unwrap_or(path).to_string(),
                file_size: 1024,
                mime_type: Some("image/jpeg".to_string()),
                taken_at: None,
                date_modified: now,
                date_indexed: Some(now),
                camera_make: None,
                camera_model: None,
                lens_make: None,
                lens_model: None,
                iso: None,
                aperture: None,
                shutter_speed: None,
                focal_length: None,
                width: Some(800),
                height: Some(600),
                color_space: None,
                white_balance: None,
                exposure_mode: None,
                metering_mode: None,
                orientation: Some(1),
                flash_used: None,
                latitude: None,
                longitude: None,
                location_name: None,

                thumbnail_path: None,
                has_thumbnail: Some(false),
                country: None,
                keywords: None,
                faces_detected: None,
                objects_detected: None,
                colors: None,
                duration: None,
                video_codec: None,
                is_favorite: Some(false),
                audio_codec: None,
                bitrate: None,
                frame_rate: None,
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
            .map(|p| p.split('/').last().unwrap_or(p).to_string())
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

        // Create scheduler with non-existent path
        let invalid_paths = vec![PathBuf::from("/nonexistent/path")];
        let scheduler = PhotoScheduler::new(invalid_paths, db_pool, cache_manager);

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
}
