use chrono::{DateTime, Utc};
use log::error;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache_manager::CacheManager;
use crate::db::DbPool;
use crate::file_scanner::{FileScanner, PhotoFile};
use crate::metadata_extractor::MetadataExtractor;
use crate::mimetype_detector;
use crate::raw_processor;
use crate::semantic_search::SemanticSearchEngine;

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
        // Process with controlled concurrency
        const MAX_CONCURRENT_TASKS: usize = crate::db_pool::MAX_CONCURRENT_PHOTO_TASKS;
        let mut tasks = tokio::task::JoinSet::new();
        let mut photos = Vec::new();
        let mut photo_files_iter = photo_files.into_iter();

        loop {
            // Fill up to MAX_CONCURRENT_TASKS
            while tasks.len() < MAX_CONCURRENT_TASKS {
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
                    processor.process_file(&photo_file).await
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

    pub async fn process_file(&self, photo_file: &PhotoFile) -> Option<ProcessedPhoto> {
        log::info!("Processing file: {}", photo_file.path.display());

        let path = &photo_file.path;
        let filename = path.file_name()?.to_string_lossy().to_string();
        let file_path = path.to_string_lossy().to_string();
        let mime_type = mimetype_detector::from_path(path).map(|m| m.to_string());
        let metadata = MetadataExtractor::extract_with_metadata(path, Some(&photo_file.metadata));
        let hash_sha256 = self.calculate_file_hash(path).ok();
        let blurhash = self.generate_blurhash(path);

        // Compute semantic vector (async for videos, sync for images)
        let is_video = mime_type
            .as_ref()
            .map(|m| m.starts_with("video/"))
            .unwrap_or(false);

        if is_video {
            // Video processing is async - await completion
            if let Err(e) = self
                .semantic_search
                .compute_video_semantic_vector(&file_path)
                .await
            {
                error!(
                    "Failed to generate video semantic vector for {}: {}",
                    file_path, e
                );
            }
        } else {
            // Image processing is synchronous
            if let Err(e) = self.semantic_search.compute_semantic_vector(&file_path) {
                error!(
                    "Failed to generate semantic vector for {}: {}",
                    file_path, e
                );
            }
        }

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
}
