//! The indexing phase: batch thumbnails through the ONNX engine, persist the
//! faces and write the People-view crops.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use log::{error, info};

use crate::cache_manager::CacheManager;
use crate::db::DbPool;
use crate::scheduler::IndexingStatus;
use crate::thumbnail_generator::ThumbnailGenerator;
use crate::thumbnail_types::{ThumbnailFormat, ThumbnailSize};

use super::engine::FaceEngine;
use super::model::MODEL_VERSION;
use super::store::{
    cleanup_orphan_vectors, clear_index_state, count_pending, old_faces, pending_photos,
    write_photo_faces,
};

const FACE_BATCH_SIZE: u32 = 10;

#[async_trait]
pub trait FaceRecognition: Send + Sync {
    fn enabled(&self) -> bool;
    async fn index_pending(&self, status: &IndexingStatus) -> Result<()>;
}

pub struct FaceRecognitionEngine {
    pool: DbPool,
    cache_manager: CacheManager,
    thumbnail_generator: ThumbnailGenerator,
    engine: Arc<FaceEngine>,
    enabled: bool,
}

impl FaceRecognitionEngine {
    pub fn new(
        pool: DbPool,
        cache_manager: CacheManager,
        thumbnail_generator: ThumbnailGenerator,
        data_path: &str,
        enabled: bool,
    ) -> Self {
        Self {
            pool,
            cache_manager,
            thumbnail_generator,
            engine: Arc::new(FaceEngine::new(data_path)),
            enabled,
        }
    }

    async fn process_photo(&self, photo: &crate::db::Photo, status: &IndexingStatus) -> Result<()> {
        status
            .phases
            .faces
            .set_current_item(Some(photo.file_path.clone()));

        let thumbnail = self
            .thumbnail_generator
            .get_or_generate(photo, ThumbnailSize::Large, ThumbnailFormat::Jpeg)
            .await
            .with_context(|| format!("Failed to build a face thumbnail for {}", photo.file_path))?;

        let engine = self.engine.clone();
        let faces = tokio::task::spawn_blocking(move || {
            let image = image::load_from_memory(&thumbnail)
                .context("Failed to decode the face thumbnail")?
                .to_rgb8();
            engine.process_image(&image)
        })
        .await
        .context("Face inference task panicked")??;

        let old = old_faces(&self.pool, &photo.hash_sha256).await?;
        // Old crops belong to face ids the reindex is about to replace. Clear
        // them before writing the new ones; a failed write leaves the face
        // rows but the next scan retries (the state row is removed below).
        self.cache_manager
            .clear_face_crops_for_hash(&photo.hash_sha256)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to clear face crops: {error}"))?;

        let crops = write_photo_faces(&self.pool, photo, &faces, &old).await?;
        for (face_id, crop) in crops {
            let path = self
                .cache_manager
                .face_crop_path(&photo.hash_sha256, face_id);
            if let Some(parent) = path.parent() {
                if let Err(error) = tokio::fs::create_dir_all(parent).await {
                    error!("Failed to create face cache directory: {error}");
                }
            }
            if let Err(error) = tokio::fs::write(&path, crop).await {
                // The row is already committed; drop its state so the next
                // scan re-detects this photo and writes the crop again.
                let _ = clear_index_state(&self.pool, &photo.hash_sha256).await;
                return Err(error)
                    .with_context(|| format!("Failed to write face crop {}", path.display()));
            }
        }

        Ok(())
    }
}

#[async_trait]
impl FaceRecognition for FaceRecognitionEngine {
    fn enabled(&self) -> bool {
        self.enabled
    }

    async fn index_pending(&self, status: &IndexingStatus) -> Result<()> {
        cleanup_orphan_vectors(&self.pool).await?;
        let total = count_pending(&self.pool, MODEL_VERSION).await?;
        status.phases.faces.set_total(total);
        if total == 0 {
            status.phases.faces.set_current_item(None);
            return Ok(());
        }

        // Load (and, on a cold cache, download) once. A failure here belongs to
        // the phase, not to every photo in the library.
        let engine = self.engine.clone();
        tokio::task::spawn_blocking(move || engine.ensure_loaded())
            .await
            .context("Face model load task panicked")??;

        info!("Phase faces: {} photo(s) need indexing", total);
        let mut after_hash = String::new();
        loop {
            let photos =
                pending_photos(&self.pool, MODEL_VERSION, &after_hash, FACE_BATCH_SIZE).await?;
            if photos.is_empty() {
                break;
            }

            for photo in &photos {
                after_hash = photo.hash_sha256.clone();
                match self.process_photo(photo, status).await {
                    Ok(()) => status.phases.faces.add_processed(1),
                    Err(error) => {
                        // Advance keyset pagination anyway: the row stays
                        // pending for the next scheduled scan, and one corrupt
                        // image must not wedge this run forever.
                        error!("Face indexing failed for {}: {}", photo.file_path, error);
                        status.phases.faces.add_errors(1);
                    }
                }
            }
        }

        status.phases.faces.set_current_item(None);
        info!(
            "Phase faces completed: {} processed, {} errors",
            status.phases.faces.snapshot().processed,
            status.phases.faces.snapshot().errors
        );
        Ok(())
    }
}

/// Test stand-in: the scheduler test doubles must not download models.
pub struct NoopFaceRecognition;

#[async_trait]
impl FaceRecognition for NoopFaceRecognition {
    fn enabled(&self) -> bool {
        false
    }

    async fn index_pending(&self, _status: &IndexingStatus) -> Result<()> {
        Ok(())
    }
}
