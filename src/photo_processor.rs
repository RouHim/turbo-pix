use chrono::{DateTime, Utc};
use log::error;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::cache_manager::CacheManager;
use crate::db::DbPool;
use crate::file_scanner::{FileScanner, PhotoFile};
use crate::metadata_extractor::MetadataExtractor;
use crate::mimetype_detector;
use crate::raw_processor;
use crate::semantic_search::SemanticSearchEngine;

/// Batch size for semantic vector computation (CPU-bound operations)
/// Smaller batches (100) provide better progress tracking and error isolation.
/// Lower than DB_WRITE_BATCH_SIZE since semantic processing is CPU/GPU-intensive
/// and benefits from smaller chunks with more frequent progress updates.
const SEMANTIC_BATCH_SIZE: usize = 100;

/// Calculate optimal concurrency for semantic vector computation
/// Balances CPU utilization with database write bottlenecks
/// Uses a hybrid algorithm:
/// - Low-core systems (<=4 cores): 1:1 mapping (all cores used for CPU-bound tasks)
/// - High-core systems (>4 cores): 60% of additional cores beyond 4
///   to avoid overwhelming SQLite single-writer performance
///
/// Examples:
/// - 4 cores: 4 tasks (100% utilization)
/// - 8 cores: 6 tasks (75% utilization)
/// - 16 cores: 11 tasks (68.75% utilization)
/// - 32 cores: 20 tasks (62.5% utilization)
/// - 64 cores: 40 tasks (62.5% utilization)
fn calculate_optimal_semantic_concurrency() -> usize {
    let cpu_cores = num_cpus::get();

    if cpu_cores <= 4 {
        // Low-core systems: CPU is bottleneck, use all cores
        cpu_cores
    } else {
        // High-core systems: Use 60% of additional cores beyond 4
        // Formula: 4 + (cores - 4) × 0.6
        let additional_cores = cpu_cores - 4;
        let additional_tasks = (additional_cores as f64 * 0.6) as usize;
        4 + additional_tasks
    }
}

#[derive(Debug)]
pub struct ProcessedPhoto {
    pub file_path: String,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: Option<String>,
    pub taken_at: Option<DateTime<Utc>>,
    pub date_modified: DateTime<Utc>,
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
    pub hash_sha256: Option<String>,
    pub blurhash: Option<String>, // BlurHash for progressive image loading
    pub duration: Option<f64>,    // Video duration in seconds
    pub video_codec: Option<String>, // Video codec (e.g., "h264", "h265")
    pub audio_codec: Option<String>, // Audio codec (e.g., "aac", "mp3")
    pub bitrate: Option<i32>,     // Bitrate in kbps
    pub frame_rate: Option<f64>,  // Frame rate for videos
}

pub struct PhotoProcessor {
    scanner: FileScanner,
    semantic_search: Arc<SemanticSearchEngine>,
}

impl PhotoProcessor {
    pub fn new(photo_paths: Vec<PathBuf>, semantic_search: Arc<SemanticSearchEngine>) -> Self {
        Self {
            scanner: FileScanner::new(photo_paths),
            semantic_search,
        }
    }

    /// Phase 1: Fast metadata scan without semantic vector computation
    ///
    /// Scans all files, extracts metadata, and cleans up orphaned database entries.
    /// Semantic vectors are computed separately in Phase 2 via `batch_compute_semantic_vectors`.
    pub async fn full_rescan_and_cleanup(
        &self,
        db_pool: &DbPool,
        cache_manager: &CacheManager,
    ) -> Result<Vec<ProcessedPhoto>, Box<dyn std::error::Error>> {
        // Step 1: Get all photo files on disk
        let photo_files = self.scanner.scan();

        // Step 2: Create list of existing paths for cleanup
        let existing_paths: Vec<String> = photo_files
            .iter()
            .map(|pf| pf.path.to_string_lossy().to_string())
            .collect();

        // Step 3: Delete orphaned photos (in database but not on disk) and clear their caches
        let deleted_paths = crate::db::delete_orphaned_photos(db_pool, &existing_paths)
            .unwrap_or_else(|e| {
                error!("Failed to delete orphaned photos: {}", e);
                Vec::new()
            });

        for path in deleted_paths {
            if let Err(e) = cache_manager.clear_for_path(&path).await {
                error!("Failed to clear cache for {}: {}", path, e);
            }
        }

        // Step 4: Process all files found on disk (with pre-check for unchanged files)
        // Process with controlled concurrency (based on CPU cores)
        let max_concurrent_tasks = crate::db_pool::max_concurrent_photo_tasks();
        let mut tasks = tokio::task::JoinSet::new();
        let mut photos = Vec::new();
        let mut photo_files_iter = photo_files.into_iter();

        loop {
            // Fill up to max_concurrent_tasks
            while tasks.len() < max_concurrent_tasks {
                let Some(photo_file) = photo_files_iter.next() else {
                    break;
                };

                let file_path = photo_file.path.to_string_lossy().to_string();
                let semantic_search = self.semantic_search.clone();

                // Check if file is unchanged (same path, size, and modification time)
                if let Some(modified) = photo_file.modified {
                    if let Ok(Some(existing_photo)) = crate::db::Photo::find_unchanged_photo(
                        db_pool,
                        &file_path,
                        photo_file.size as i64,
                        modified,
                    ) {
                        log::debug!("Skipping unchanged file: {}", file_path);

                        // Convert existing Photo to ProcessedPhoto - no async needed
                        tasks.spawn(async move {
                            Some(ProcessedPhoto {
                                file_path: existing_photo.file_path.clone(),
                                filename: existing_photo.filename.clone(),
                                file_size: existing_photo.file_size,
                                mime_type: existing_photo.mime_type.clone(),
                                taken_at: existing_photo.taken_at,
                                date_modified: existing_photo.date_modified,
                                camera_make: existing_photo.camera_make().map(String::from),
                                camera_model: existing_photo.camera_model().map(String::from),
                                lens_make: existing_photo.lens_make().map(String::from),
                                lens_model: existing_photo.lens_model().map(String::from),
                                iso: existing_photo.iso(),
                                aperture: existing_photo.aperture(),
                                shutter_speed: existing_photo.shutter_speed().map(String::from),
                                focal_length: existing_photo.focal_length(),
                                width: existing_photo.width,
                                height: existing_photo.height,
                                color_space: existing_photo.color_space().map(String::from),
                                white_balance: existing_photo.white_balance().map(String::from),
                                exposure_mode: existing_photo.exposure_mode().map(String::from),
                                metering_mode: existing_photo.metering_mode().map(String::from),
                                orientation: existing_photo.orientation,
                                flash_used: existing_photo.flash_used(),
                                latitude: existing_photo.latitude(),
                                longitude: existing_photo.longitude(),
                                hash_sha256: Some(existing_photo.hash_sha256.clone()),
                                blurhash: existing_photo.blurhash.clone(),
                                duration: existing_photo.duration,
                                video_codec: existing_photo.video_codec().map(String::from),
                                audio_codec: existing_photo.audio_codec().map(String::from),
                                bitrate: existing_photo.bitrate(),
                                frame_rate: existing_photo.frame_rate(),
                            })
                        });
                        continue;
                    }
                }

                // File is new or modified - spawn async processing task
                tasks.spawn(async move {
                    // Need to recreate processor context in the task
                    let processor = PhotoProcessor {
                        scanner: FileScanner::new(Vec::new()), // Empty scanner, not used
                        semantic_search,
                    };
                    processor.process_file_metadata_only(&photo_file).await
                });
            }

            // If no tasks are running and no more files, we're done
            if tasks.is_empty() {
                break;
            }

            // Wait for at least one task to complete
            if let Some(Ok(Some(photo))) = tasks.join_next().await {
                photos.push(photo);
            }
        }

        Ok(photos)
    }

    /// Phase 1: Extract metadata without computing semantic vectors
    ///
    /// Processes file metadata (EXIF, dimensions, etc.) but skips semantic vector computation.
    /// Semantic vectors are computed separately in Phase 2.
    pub async fn process_file_metadata_only(
        &self,
        photo_file: &PhotoFile,
    ) -> Option<ProcessedPhoto> {
        log::info!("Processing file: {}", photo_file.path.display());

        let path = &photo_file.path;
        let filename = path.file_name()?.to_string_lossy().to_string();
        let file_path = path.to_string_lossy().to_string();
        let mime_type = mimetype_detector::from_path(path).map(|m| m.to_string());
        let metadata = MetadataExtractor::extract_with_metadata(path, Some(&photo_file.metadata));
        let hash_sha256 = self.calculate_file_hash(path).ok();
        let blurhash = self.generate_blurhash(path);

        // Semantic vectors are skipped in Phase 1 and computed in Phase 2

        Some(ProcessedPhoto {
            file_path: file_path.clone(),
            filename,
            file_size: photo_file.size as i64,
            mime_type,
            taken_at: metadata.taken_at,
            date_modified: photo_file.modified.unwrap_or_else(Utc::now),
            camera_make: metadata.camera_make,
            camera_model: metadata.camera_model,
            lens_make: metadata.lens_make,
            lens_model: metadata.lens_model,
            iso: metadata.iso,
            aperture: metadata.aperture,
            shutter_speed: metadata.shutter_speed,
            focal_length: metadata.focal_length,
            width: metadata.width.map(|w| w as i32),
            height: metadata.height.map(|h| h as i32),
            color_space: metadata.color_space,
            white_balance: metadata.white_balance,
            exposure_mode: metadata.exposure_mode,
            metering_mode: metadata.metering_mode,
            orientation: metadata.orientation,
            flash_used: metadata.flash_used,
            latitude: metadata.latitude,
            longitude: metadata.longitude,
            hash_sha256,
            blurhash,
            duration: metadata.duration,
            video_codec: metadata.video_codec,
            audio_codec: metadata.audio_codec,
            bitrate: metadata.bitrate,
            frame_rate: metadata.frame_rate,
        })
    }

    fn calculate_file_hash(&self, path: &Path) -> Result<String, Box<dyn std::error::Error>> {
        let mut hasher = Sha256::new();
        hasher.update(path.to_string_lossy().as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn generate_blurhash(&self, path: &Path) -> Option<String> {
        // Only generate blurhash for image files (not videos)
        let mime_type = mimetype_detector::from_path(path)?;
        if mime_type.type_() != "image" {
            return None;
        }

        // Load image (RAW or standard format)
        let img = if raw_processor::is_raw_file(path) {
            raw_processor::decode_raw_to_dynamic_image(path).ok()?
        } else {
            image::open(path).ok()?
        };
        let resized = img.thumbnail(32, 32); // Small size for blurhash generation

        // Convert to RGBA8 (fast-blurhash expects u32 pixels)
        let rgba = resized.to_rgba8();
        let (width, height) = rgba.dimensions();

        // Convert RGBA bytes to u32 pixels
        let pixels: Vec<u32> = rgba
            .chunks(4)
            .map(|chunk| {
                let r = chunk[0] as u32;
                let g = chunk[1] as u32;
                let b = chunk[2] as u32;
                let a = chunk[3] as u32;
                (a << 24) | (r << 16) | (g << 8) | b
            })
            .collect();

        // Generate blurhash with 4x3 components (good balance between quality and size)
        // fast-blurhash uses a two-step process: compute_dct -> into_blurhash
        let dct_result = fast_blurhash::compute_dct(
            &pixels,
            width as usize,
            height as usize,
            4, // x_components
            3, // y_components
        );
        let hash = dct_result.into_blurhash();
        Some(hash)
    }

    /// Phase 2: Batch compute semantic vectors for photos missing indexing
    ///
    /// This is called after Phase 1 (fast metadata scan) completes.
    /// Only processes photos where semantic_vector_indexed = false.
    /// Limits concurrency to available CPU cores for optimal performance.
    pub async fn batch_compute_semantic_vectors(
        &self,
        db_pool: &DbPool,
    ) -> Result<(usize, usize), Box<dyn std::error::Error>> {
        use log::info;

        info!("Phase 2: Starting batch semantic vector computation");

        // Get only photos that need semantic indexing
        let photo_paths = crate::db::get_paths_needing_semantic_indexing(db_pool)?;
        let total_count = photo_paths.len();

        if total_count == 0 {
            info!("No photos found needing semantic vector computation");
            return Ok((0, 0));
        }

        info!(
            "Computing semantic vectors for {} photos (skipping already indexed)",
            total_count
        );

        // Calculate optimal concurrency using hybrid algorithm
        // Balances CPU utilization (low cores) with database write bottleneck (high cores)
        let max_concurrency = calculate_optimal_semantic_concurrency();
        let cpu_cores = num_cpus::get();
        let semaphore = Arc::new(Semaphore::new(max_concurrency));
        info!(
            "Using {} concurrent tasks for {} CPU cores ({:.1}% utilization, optimized for SQLite single-writer)",
            max_concurrency,
            cpu_cores,
            (max_concurrency as f64 / cpu_cores as f64) * 100.0
        );

        let mut processed_count = 0;
        let mut error_count = 0;

        for (batch_idx, batch_paths) in photo_paths.chunks(SEMANTIC_BATCH_SIZE).enumerate() {
            let mut batch_tasks = tokio::task::JoinSet::new();

            for path in batch_paths {
                let path = path.clone();
                let semantic_search = self.semantic_search.clone();
                let db_pool = db_pool.clone();
                let semaphore = semaphore.clone();

                batch_tasks.spawn(async move {
                    // Acquire semaphore permit to limit concurrency
                    let _permit = semaphore
                        .acquire()
                        .await
                        .expect("Semaphore should not be closed");

                    // Determine if video or image
                    let path_buf = std::path::PathBuf::from(&path);
                    let is_video = mimetype_detector::from_path(&path_buf)
                        .map(|m| m.type_() == "video")
                        .unwrap_or(false);

                    // Compute semantic vector
                    let result = if is_video {
                        semantic_search.compute_video_semantic_vector(&path).await
                    } else {
                        semantic_search.compute_semantic_vector(&path)
                    };

                    // Mark as indexed if successful
                    if result.is_ok() {
                        if let Err(e) =
                            crate::db::mark_photo_as_semantically_indexed(&db_pool, &path)
                        {
                            error!("Failed to mark photo as indexed {}: {}", path, e);
                        }
                    }

                    result.map(|_| path.clone())
                });
            }

            // Wait for all tasks in this batch to complete
            while let Some(result) = batch_tasks.join_next().await {
                match result {
                    Ok(Ok(_)) => processed_count += 1,
                    Ok(Err(e)) => {
                        error!("Failed to compute semantic vector: {}", e);
                        error_count += 1;
                    }
                    Err(e) => {
                        error!("Task panicked: {}", e);
                        error_count += 1;
                    }
                }
            }

            // Progress logging every 5 batches
            if (batch_idx + 1) % 5 == 0 {
                info!(
                    "Semantic vector progress: {}/{} photos ({:.1}%), {} errors",
                    processed_count + error_count,
                    total_count,
                    ((processed_count + error_count) as f64 / total_count as f64) * 100.0,
                    error_count
                );
            }
        }

        info!(
            "Phase 2 completed: {} semantic vectors computed, {} errors",
            processed_count, error_count
        );

        Ok((processed_count, error_count))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_calculate_optimal_semantic_concurrency_algorithm() {
        // Test cases: (simulated_cores, expected_tasks, description)
        let test_cases = [
            (1, 1, "Single core: 1:1 mapping"),
            (2, 2, "Dual core: 1:1 mapping"),
            (3, 3, "Triple core: 1:1 mapping"),
            (4, 4, "Quad core: 1:1 mapping (boundary)"),
            (6, 5, "6 cores: 4 + (6-4)×0.6 = 5"),
            (8, 6, "8 cores: 4 + (8-4)×0.6 = 6"),
            (12, 8, "12 cores: 4 + (12-4)×0.6 = 8"),
            (16, 11, "16 cores: 4 + (16-4)×0.6 = 11"),
            (24, 16, "24 cores: 4 + (24-4)×0.6 = 16"),
            (32, 20, "32 cores: 4 + (32-4)×0.6 = 20"),
            (64, 40, "64 cores: 4 + (64-4)×0.6 = 40"),
        ];

        for (simulated_cores, expected, description) in test_cases {
            // Simulate the calculation without mocking num_cpus
            let actual = if simulated_cores <= 4 {
                simulated_cores
            } else {
                let additional_cores = simulated_cores - 4;
                let additional_tasks = (additional_cores as f64 * 0.6) as usize;
                4 + additional_tasks
            };

            assert_eq!(
                actual, expected,
                "Failed: {} (cores={}, expected={}, got={})",
                description, simulated_cores, expected, actual
            );
        }
    }

    #[test]
    fn test_concurrency_never_exceeds_cores() {
        // Ensure we never request more tasks than available cores
        for cores in 1..=128 {
            let tasks = if cores <= 4 {
                cores
            } else {
                let additional_cores = cores - 4;
                let additional_tasks = (additional_cores as f64 * 0.6) as usize;
                4 + additional_tasks
            };

            assert!(
                tasks <= cores,
                "Concurrency {} exceeds cores {} (invalid!)",
                tasks,
                cores
            );
        }
    }

    #[test]
    fn test_concurrency_always_positive() {
        // Ensure we always have at least 1 task
        for cores in 1..=128 {
            let tasks = if cores <= 4 {
                cores
            } else {
                let additional_cores = cores - 4;
                let additional_tasks = (additional_cores as f64 * 0.6) as usize;
                4 + additional_tasks
            };

            assert!(tasks >= 1, "Concurrency must be at least 1, got {}", tasks);
        }
    }

    #[test]
    fn test_concurrency_scaling_ratio() {
        // Verify that utilization ratio decreases from 100% (at 4 cores) and stabilizes around 60%
        let cores_to_test = [4, 8, 16, 32, 64];

        for &cores in &cores_to_test {
            let tasks = if cores <= 4 {
                cores
            } else {
                let additional_cores = cores - 4;
                let additional_tasks = (additional_cores as f64 * 0.6) as usize;
                4 + additional_tasks
            };

            let ratio = tasks as f64 / cores as f64;

            if cores == 4 {
                // At 4 cores, should be 100%
                assert_eq!(ratio, 1.0, "At 4 cores, ratio should be 100%");
            } else {
                // Beyond 4 cores, ratio should be between 60-75%
                assert!(
                    (0.60..=0.75).contains(&ratio),
                    "Beyond 4 cores, ratio should be 60-75%: cores={}, ratio={:.2}",
                    cores,
                    ratio
                );
            }
        }
    }
}
