use chrono::{DateTime, Utc};
use log::{error, info};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::CacheManager;
use crate::db::{DbPool, Photo};
use crate::file_scanner::{FileScanner, PhotoFile};
use crate::metadata_extractor::MetadataExtractor;
use crate::mimetype_detector;
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
    pub duration: Option<f64>,       // Video duration in seconds
    pub video_codec: Option<String>, // Video codec (e.g., "h264", "h265")
    pub audio_codec: Option<String>, // Audio codec (e.g., "aac", "mp3")
    pub bitrate: Option<i32>,        // Bitrate in kbps
    pub frame_rate: Option<f64>,     // Frame rate for videos
}

impl ProcessedPhoto {
    #[allow(dead_code)]
    pub fn save_to_db(&self, db_pool: &DbPool) -> Result<(), Box<dyn std::error::Error>> {
        let photo = Photo {
            hash_sha256: self
                .hash_sha256
                .clone()
                .expect("ProcessedPhoto must have hash_sha256"),
            file_path: self.file_path.clone(),
            filename: self.filename.clone(),
            file_size: self.file_size,
            mime_type: self.mime_type.clone(),
            taken_at: self.taken_at,
            date_modified: self.date_modified,
            date_indexed: Some(Utc::now()),
            camera_make: self.camera_make.clone(),
            camera_model: self.camera_model.clone(),
            lens_make: self.lens_make.clone(),
            lens_model: self.lens_model.clone(),
            iso: self.iso,
            aperture: self.aperture,
            shutter_speed: self.shutter_speed.clone(),
            focal_length: self.focal_length,
            width: self.width,
            height: self.height,
            color_space: self.color_space.clone(),
            white_balance: self.white_balance.clone(),
            exposure_mode: self.exposure_mode.clone(),
            metering_mode: self.metering_mode.clone(),
            orientation: self.orientation,
            flash_used: self.flash_used,
            latitude: self.latitude,
            longitude: self.longitude,
            location_name: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            country: None,
            keywords: None,
            faces_detected: None,
            objects_detected: None,
            colors: None,
            duration: self.duration,
            video_codec: self.video_codec.clone(),
            audio_codec: self.audio_codec.clone(),
            bitrate: self.bitrate,
            frame_rate: self.frame_rate,
            is_favorite: Some(false),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        photo.create(db_pool)
    }
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

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn process_all(&self) -> Vec<ProcessedPhoto> {
        let photo_files = self.scanner.scan();
        let mut processed_photos = Vec::new();

        for photo_file in photo_files {
            if let Some(processed_photo) = self.process_file(&photo_file) {
                processed_photos.push(processed_photo);
            }
        }

        processed_photos
    }

    #[allow(dead_code)]
    pub async fn process_new_photos(
        &self,
        db_pool: &DbPool,
        cache_manager: &CacheManager,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let photo_files = self.scanner.scan();
        let existing_photos = crate::db::get_all_photo_paths(db_pool)?;
        let existing_set: HashSet<String> = existing_photos.into_iter().collect();

        let new_photos: Vec<_> = photo_files
            .into_iter()
            .filter(|photo| !existing_set.contains(&photo.path.to_string_lossy().to_string()))
            .collect();

        if new_photos.is_empty() {
            info!("No new photos found");
            return Ok(0);
        }

        info!("Processing {} new photos...", new_photos.len());

        let mut processed_count = 0;
        for photo_file in new_photos {
            match self
                .process_and_store(&photo_file, db_pool, cache_manager)
                .await
            {
                Ok(_) => processed_count += 1,
                Err(e) => error!(
                    "Failed to process photo {}: {}",
                    photo_file.path.display(),
                    e
                ),
            }
        }

        Ok(processed_count)
    }

    pub async fn full_rescan_and_cleanup(
        &self,
        db_pool: &DbPool,
        _cache_manager: &CacheManager,
    ) -> Result<Vec<ProcessedPhoto>, Box<dyn std::error::Error>> {
        // Step 1: Get all photo files on disk
        let photo_files = self.scanner.scan();

        // Step 2: Create list of existing paths for cleanup
        let existing_paths: Vec<String> = photo_files
            .iter()
            .map(|pf| pf.path.to_string_lossy().to_string())
            .collect();

        // Step 3: Delete orphaned photos (in database but not on disk)
        if let Err(e) = crate::db::delete_orphaned_photos(db_pool, &existing_paths) {
            eprintln!("Failed to delete orphaned photos: {}", e);
        }

        // Step 4: Process all files found on disk (parallel processing)
        let photos: Vec<ProcessedPhoto> = photo_files
            .par_iter()
            .filter_map(|photo_file| self.process_file(photo_file))
            .collect();

        Ok(photos)
    }

    #[allow(dead_code)]
    async fn process_and_store(
        &self,
        photo_file: &PhotoFile,
        db_pool: &DbPool,
        _cache_manager: &CacheManager,
    ) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(processed_photo) = self.process_file(photo_file) {
            let hash = processed_photo
                .hash_sha256
                .clone()
                .ok_or("Missing hash for processed photo")?;
            processed_photo.save_to_db(db_pool)?;
            Ok(hash)
        } else {
            Err("Failed to process photo file".into())
        }
    }

    pub fn process_file(&self, photo_file: &PhotoFile) -> Option<ProcessedPhoto> {
        log::info!("Processing file: {}", photo_file.path.display());

        log::info!("\t* Computing metadata...");
        let path = &photo_file.path;
        let filename = path.file_name()?.to_string_lossy().to_string();
        let file_path = path.to_string_lossy().to_string();
        let mime_type = mimetype_detector::from_path(path).map(|m| m.to_string());
        let metadata = MetadataExtractor::extract_with_metadata(path, Some(&photo_file.metadata));
        let hash_sha256 = self.calculate_file_hash(path).ok();

        self.semantic_search
            .compute_semantic_vector(&file_path)
            .ok();

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
}
