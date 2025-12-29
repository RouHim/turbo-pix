//! Semantic Image Search
//!
//! This module integrates OpenAI's CLIP model for semantic image search in TurboPix.
//! Image semantic vectors are cached in SQLite using sqlite-vec for efficient reuse.
//!
//! ## Architecture
//!
//! 1. **Model**: CLIP ViT-B/32 from HuggingFace (512-dim semantic vectors)
//! 2. **Cache**: SQLite with sqlite-vec extension for vector storage
//! 3. **Pipeline**: Text query → encode → compare with cached image semantic vectors → rank
//!
//! ## Score Interpretation (0-100 scale)
//!
//! Scores are normalized to a 0-100 scale for user-facing display:
//! - **70+**: Excellent match (strong semantic relevance)
//! - **60-70**: Good match (clear semantic relationship)
//! - **50-60**: Weak match (loose semantic connection)
//! - **< 50**: Filtered out (insufficient relevance)
//!
//! Note: CLIP baseline behavior produces scores around 50-60 for unrelated images.
//! Relevant matches typically score 60-70+, with strong matches reaching 70+.

use anyhow::{Context, Error as E, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::clip;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokenizers::Tokenizer;
use zerocopy::IntoBytes;

use crate::raw_processor;

// Model configuration
const CLIP_MODEL: &str = "openai/clip-vit-base-patch32";
const MODEL_REVISION: &str = "d15b5f29721ca72dac15f8526b284be910de18be";

// Text encoding parameters
const CONTEXT_LENGTH: usize = 77;
const EOT_TOKEN: u32 = 49407;

// Image preprocessing constants
const CLIP_IMAGE_SIZE: u32 = 224;
const CLIP_MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const CLIP_STD: [f32; 3] = [0.26862954, 0.261_302_6, 0.275_777_1];

// Minimum similarity score threshold for search results (0.0-1.0 range)
// Normalized to 0-100 scale for user display (multiply by 100)
const MIN_SIMILARITY_SCORE: f32 = 0.615;

// Video frame sampling configuration
const VIDEO_FRAME_COUNT: usize = 3;
const MODEL_VERSION: &str = "clip-vit-base-patch32-v1";

/// Semantic search engine for image search
pub struct SemanticSearchEngine {
    model: Arc<RwLock<clip::ClipModel>>,
    tokenizer: Arc<Tokenizer>,
    device: Arc<Device>,
    pool: Pool<SqliteConnectionManager>,
}

impl SemanticSearchEngine {
    /// Creates a new semantic search engine with model and database initialization
    pub fn new(db_pool: Pool<SqliteConnectionManager>, data_path: &str) -> Result<Self> {
        let device = Arc::new(Device::Cpu);

        log::info!("Loading semantic search model...");
        let (model, tokenizer) = load_clip_model(&device, data_path)?;

        Ok(Self {
            model: Arc::new(RwLock::new(model)),
            tokenizer: Arc::new(tokenizer),
            device,
            pool: db_pool,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_with_model(
        model: Arc<RwLock<clip::ClipModel>>,
        tokenizer: Arc<Tokenizer>,
        device: Arc<Device>,
        pool: Pool<SqliteConnectionManager>,
    ) -> Self {
        Self {
            model,
            tokenizer,
            device,
            pool,
        }
    }

    /// Retry helper for database operations with exponential backoff
    /// Handles "database is locked" errors during high concurrent write load
    fn retry_on_db_locked<F, T>(&self, operation: F, operation_name: &str) -> Result<T>
    where
        F: Fn() -> Result<T>,
    {
        const MAX_RETRIES: u32 = 5;
        const BASE_DELAY_MS: u64 = 100;

        for attempt in 0..MAX_RETRIES {
            match operation() {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let err_msg = e.to_string();
                    if err_msg.contains("database is locked") && attempt < MAX_RETRIES - 1 {
                        let delay_ms = BASE_DELAY_MS * 2_u64.pow(attempt);
                        log::warn!(
                            "Database locked during {} (attempt {}/{}), retrying in {}ms",
                            operation_name,
                            attempt + 1,
                            MAX_RETRIES,
                            delay_ms
                        );
                        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Err(anyhow::anyhow!(
            "Failed after {} retries: {}",
            MAX_RETRIES,
            operation_name
        ))
    }

    /// Performs semantic search for a text query across all images using sqlite-vec KNN
    pub fn search(&self, query: &str, limit: usize, offset: usize) -> Result<Vec<(String, f32)>> {
        let start_time = std::time::Instant::now();
        log::info!(
            "Semantic search for: '{}' (limit: {}, offset: {})",
            query,
            limit,
            offset
        );

        // Encode text query to semantic vector
        let encode_start = std::time::Instant::now();
        let text_semantic_vector = {
            let model_read = self
                .model
                .read()
                .map_err(|e| anyhow::anyhow!("Failed to acquire model lock: {}", e))?;
            encode_text(&model_read, &self.tokenizer, query, &self.device)?
        };
        log::debug!("Text encoding took: {:?}", encode_start.elapsed());

        // Convert semantic vector to bytes for sqlite-vec
        let vector_floats: Vec<f32> = text_semantic_vector.flatten_all()?.to_vec1()?;
        let vector_bytes = vector_floats.as_slice().as_bytes().to_vec();

        // Use sqlite-vec's built-in KNN search
        // vec_distance_cosine returns distance (0 = identical, 2 = opposite)
        // Convert to similarity score (1 = identical, 0 = orthogonal, -1 = opposite)
        let db_start = std::time::Instant::now();
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT ic.path, 1.0 - (vec_distance_cosine(msv.semantic_vector, ?) / 2.0) as similarity
             FROM media_semantic_vectors msv
             JOIN semantic_vector_path_mapping ic ON msv.rowid = ic.id
             ORDER BY vec_distance_cosine(msv.semantic_vector, ?)
             LIMIT ? OFFSET ?",
        )?;

        let results: Vec<(String, f32)> = stmt
            .query_map(
                rusqlite::params![&vector_bytes, &vector_bytes, limit as i64, offset as i64],
                |row| {
                    let path: String = row.get(0)?;
                    let score: f32 = row.get(1)?;
                    Ok((path, score))
                },
            )?
            .filter_map(|r| r.ok())
            .filter(|(_, score)| *score >= MIN_SIMILARITY_SCORE)
            .map(|(path, score)| (path, score * 100.0))
            .collect();
        log::debug!("Database query took: {:?}", db_start.elapsed());

        log::info!("Semantic search results for '{}':", query);
        for (path, score) in &results {
            log::info!("  {:.1} - {}", score, path);
        }
        log::info!(
            "Total: {} results (filtered by score >= {:.0})",
            results.len(),
            MIN_SIMILARITY_SCORE * 100.0
        );
        log::debug!("Total search time: {:?}", start_time.elapsed());

        Ok(results)
    }

    /// Computes and caches semantic vector for a single image
    pub fn compute_semantic_vector(&self, image_path: &str) -> Result<()> {
        // OPTIMIZATION: Check existence BEFORE acquiring transaction lock
        // WAL mode allows concurrent reads without blocking writers
        let conn = self.pool.get()?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
                [image_path],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if exists {
            log::debug!("Semantic vector already cached for: {}", image_path);
            return Ok(());
        }
        drop(conn);

        // OPTIMIZATION: Compute expensive vector BEFORE acquiring write lock
        let model_read = self
            .model
            .read()
            .map_err(|e| anyhow::anyhow!("Failed to acquire model lock: {}", e))?;
        let semantic_vector = encode_image(&model_read, image_path, &self.device)?;
        drop(model_read);

        // OPTIMIZATION: Retry database write with exponential backoff
        let pool = &self.pool;
        let path = image_path;
        self.retry_on_db_locked(
            || {
                // OPTIMIZATION: Only hold IMMEDIATE transaction for minimal write operation
                // Use IMMEDIATE to prevent race condition where multiple tasks compute same vector
                let mut conn = pool.get()?;
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

                // Double-check existence after expensive computation (race condition prevention)
                let still_missing: bool = tx
                    .query_row(
                        "SELECT NOT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
                        [path],
                        |row| row.get(0),
                    )
                    .unwrap_or(true);

                if still_missing {
                    store_semantic_vector_tx(&tx, path, &semantic_vector)?;
                } else {
                    log::debug!(
                        "Semantic vector was computed by another task for: {}",
                        path
                    );
                }

                tx.commit()?;
                Ok(())
            },
            "semantic vector storage",
        )
    }

    /// Computes and caches semantic vector for a video by sampling frames
    pub async fn compute_video_semantic_vector(
        &self,
        video_path: &str,
        frame_count: Option<usize>,
    ) -> Result<()> {
        use crate::video_processor::{extract_frames_batch, extract_video_metadata};

        // Quick check without holding a transaction (to avoid holding lock across async)
        let conn = self.pool.get()?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
                [video_path],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if exists {
            log::debug!("Semantic vector already cached for video: {}", video_path);
            return Ok(());
        }
        drop(conn); // Release connection before async work

        log::info!("Computing semantic vector for video: {}", video_path);
        let start_time = std::time::Instant::now();
        let frames_to_sample = frame_count.unwrap_or(VIDEO_FRAME_COUNT);

        // Extract video metadata
        let metadata = extract_video_metadata(Path::new(video_path)).await?;
        let frame_times = calculate_frame_positions(metadata.duration, frames_to_sample);

        log::debug!(
            "Sampling {} frames at positions: {:?}",
            frame_times.len(),
            frame_times
        );

        // Create temp directory for frame extraction
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir =
            std::env::temp_dir().join(format!("turbopix_{}_{}", std::process::id(), unique_id));

        // Step 1: Extract all frames in a single ffmpeg call (MUCH faster than parallel tasks)
        let extraction_start = std::time::Instant::now();
        let extracted_frames =
            extract_frames_batch(Path::new(video_path), &frame_times, &temp_dir).await?;
        log::debug!(
            "Extracted {} frames in {:?}",
            extracted_frames.len(),
            extraction_start.elapsed()
        );

        // Step 2: Encode frames to embeddings (batch inference)
        let model_read = self
            .model
            .read()
            .map_err(|e| anyhow::anyhow!("Failed to acquire model lock: {}", e))?;

        let batch_embeddings = encode_image_batch(&model_read, &extracted_frames, &self.device)?;
        drop(model_read);

        // Step 3: Cleanup temp files
        for temp_frame_path in &extracted_frames {
            if let Err(e) = std::fs::remove_file(temp_frame_path) {
                log::warn!(
                    "Failed to cleanup temp frame {}: {}",
                    temp_frame_path.display(),
                    e
                );
            }
        }

        // Cleanup temp directory
        if let Err(e) = std::fs::remove_dir(&temp_dir) {
            log::warn!(
                "Failed to cleanup temp directory {}: {}",
                temp_dir.display(),
                e
            );
        }

        // Average pool all frame embeddings: [N, 512] -> [512]
        let video_embedding = batch_embeddings.mean(0)?;
        let normalized_embedding = normalize_vector(&video_embedding)?;

        // Store embedding and metadata in a transaction (after all async work is done)
        // Retry with exponential backoff for database locked errors
        let pool = &self.pool;
        let path = video_path;
        self.retry_on_db_locked(
            || {
                let mut conn = pool.get()?;
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

                // Double-check it wasn't inserted by another concurrent task
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
                        [path],
                        |row| row.get(0),
                    )
                    .unwrap_or(false);

                if !exists {
                    store_semantic_vector_tx(&tx, path, &normalized_embedding)?;
                    store_video_metadata_tx(
                        &tx,
                        path,
                        frames_to_sample,
                        &frame_times,
                        MODEL_VERSION,
                    )?;
                } else {
                    log::debug!(
                        "Video semantic vector was cached by another task during computation: {}",
                        path
                    );
                }

                tx.commit()?;
                Ok(())
            },
            "video semantic vector storage",
        )?;

        log::info!(
            "Video semantic vector computed in {:?}: {}",
            start_time.elapsed(),
            video_path
        );

        Ok(())
    }
}

/// Downloads CLIP model files to the cache directory
pub fn download_models(data_path: &str) -> Result<()> {
    log::info!("Downloading CLIP model to cache...");
    let cache_dir = std::path::PathBuf::from(data_path).join("../data/models");

    let model_repo = hf_hub::api::sync::ApiBuilder::new()
        .with_cache_dir(cache_dir.clone())
        .build()
        .context("Failed to build HuggingFace API")?
        .repo(hf_hub::Repo::with_revision(
            CLIP_MODEL.into(),
            hf_hub::RepoType::Model,
            MODEL_REVISION.into(),
        ));

    log::info!("Downloading model weights...");
    let weights_path = model_repo
        .get("model.safetensors")
        .context("Failed to download model weights")?;
    log::info!("Model weights downloaded: {}", weights_path.display());

    log::info!("Downloading tokenizer...");
    let tokenizer_path = model_repo
        .get("tokenizer.json")
        .context("Failed to download tokenizer")?;
    log::info!("Tokenizer downloaded: {}", tokenizer_path.display());

    log::info!(
        "All models downloaded successfully to: {}",
        cache_dir.display()
    );
    Ok(())
}

/// Loads CLIP ViT-B/32 model and tokenizer from HuggingFace Hub
fn load_clip_model(device: &Device, data_path: &str) -> Result<(clip::ClipModel, Tokenizer)> {
    let cache_dir = std::path::PathBuf::from(data_path).join("../data/models");

    let model_repo = hf_hub::api::sync::ApiBuilder::new()
        .with_cache_dir(cache_dir)
        .build()
        .context("Failed to build HuggingFace API")?
        .repo(hf_hub::Repo::with_revision(
            CLIP_MODEL.into(),
            hf_hub::RepoType::Model,
            MODEL_REVISION.into(),
        ));

    let weights_filename = model_repo
        .get("model.safetensors")
        .context("Failed to download model weights")?;
    let tokenizer_filename = model_repo
        .get("tokenizer.json")
        .context("Failed to download tokenizer")?;

    let config = clip::ClipConfig::vit_base_patch32();
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[weights_filename], DType::F32, device)
            .context("Failed to load safetensors")?
    };

    let model = clip::ClipModel::new(vb, &config)?;
    let tokenizer = Tokenizer::from_file(tokenizer_filename)
        .map_err(E::msg)
        .context("Failed to load tokenizer")?;

    Ok((model, tokenizer))
}

/// Encodes text into a normalized 512-dimensional semantic vector
fn encode_text(
    model: &clip::ClipModel,
    tokenizer: &Tokenizer,
    text: &str,
    device: &Device,
) -> Result<Tensor> {
    let mut tokens = tokenizer
        .encode(text, true)
        .map_err(E::msg)?
        .get_ids()
        .to_vec();

    if tokens.len() < CONTEXT_LENGTH {
        tokens.resize(CONTEXT_LENGTH, EOT_TOKEN);
    } else {
        tokens.truncate(CONTEXT_LENGTH);
    }

    let token_ids = Tensor::new(&tokens[..], device)?.unsqueeze(0)?;
    let features = model.get_text_features(&token_ids)?;
    normalize_vector(&features)
}

/// Encodes an image into a normalized 512-dimensional semantic vector
fn encode_image(model: &clip::ClipModel, image_path: &str, device: &Device) -> Result<Tensor> {
    let path = Path::new(image_path);

    let img = if raw_processor::is_raw_file(path) {
        raw_processor::decode_raw_to_dynamic_image(path)
            .context("Failed to decode RAW file for CLIP encoding")?
    } else {
        image::open(path)?
    };

    let img = if img.width() == CLIP_IMAGE_SIZE && img.height() == CLIP_IMAGE_SIZE {
        img
    } else {
        img.resize_exact(
            CLIP_IMAGE_SIZE,
            CLIP_IMAGE_SIZE,
            image::imageops::FilterType::Triangle,
        )
    };

    let img_rgb = img.to_rgb8();

    let data: Vec<f32> = img_rgb
        .pixels()
        .flat_map(|p| {
            [
                (p[0] as f32) / 255.0,
                (p[1] as f32) / 255.0,
                (p[2] as f32) / 255.0,
            ]
        })
        .collect();

    let img_tensor = Tensor::from_vec(
        data,
        (CLIP_IMAGE_SIZE as usize, CLIP_IMAGE_SIZE as usize, 3),
        device,
    )?
    .permute((2, 0, 1))?
    .unsqueeze(0)?;

    let mean = Tensor::new(&CLIP_MEAN, device)?.reshape((1, 3, 1, 1))?;
    let std = Tensor::new(&CLIP_STD, device)?.reshape((1, 3, 1, 1))?;
    let img_normalized = img_tensor.broadcast_sub(&mean)?.broadcast_div(&std)?;

    let features = model.get_image_features(&img_normalized)?;
    normalize_vector(&features)
}

/// Encodes a batch of images into normalized 512-dimensional semantic vectors
fn encode_image_batch(
    model: &clip::ClipModel,
    image_paths: &[std::path::PathBuf],
    device: &Device,
) -> Result<Tensor> {
    use rayon::prelude::*;

    let tensors: Vec<Tensor> = image_paths
        .par_iter()
        .map(|path| {
            let img = if raw_processor::is_raw_file(path) {
                raw_processor::decode_raw_to_dynamic_image(path)
                    .context("Failed to decode RAW file for CLIP encoding")?
            } else {
                image::open(path)?
            };

            let img = if img.width() == CLIP_IMAGE_SIZE && img.height() == CLIP_IMAGE_SIZE {
                img
            } else {
                img.resize_exact(
                    CLIP_IMAGE_SIZE,
                    CLIP_IMAGE_SIZE,
                    image::imageops::FilterType::Triangle,
                )
            };

            let img_rgb = img.to_rgb8();

            let data: Vec<f32> = img_rgb
                .pixels()
                .flat_map(|p| {
                    [
                        (p[0] as f32) / 255.0,
                        (p[1] as f32) / 255.0,
                        (p[2] as f32) / 255.0,
                    ]
                })
                .collect();

            let img_tensor = Tensor::from_vec(
                data,
                (CLIP_IMAGE_SIZE as usize, CLIP_IMAGE_SIZE as usize, 3),
                &Device::Cpu, // Always decode on CPU
            )?
            .permute((2, 0, 1))?;

            Ok(img_tensor)
        })
        .collect::<Result<Vec<Tensor>>>()?;

    if tensors.is_empty() {
        return Err(anyhow::anyhow!("Cannot encode empty batch"));
    }

    // Move tensors to the target device if necessary
    let tensors: Vec<Tensor> = tensors
        .into_iter()
        .map(|t| t.to_device(device))
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Stack: [B, 3, 224, 224]
    let batch = Tensor::stack(&tensors, 0)?;

    let mean = Tensor::new(&CLIP_MEAN, device)?.reshape((1, 3, 1, 1))?;
    let std = Tensor::new(&CLIP_STD, device)?.reshape((1, 3, 1, 1))?;
    let batch_normalized = batch.broadcast_sub(&mean)?.broadcast_div(&std)?;

    let features = model.get_image_features(&batch_normalized)?;
    normalize_vector(&features)
}

/// Computes L2 normalization of a semantic vector
fn normalize_vector(vector: &Tensor) -> Result<Tensor> {
    let dim = match vector.rank() {
        1 => 0,
        2 => 1,
        r => {
            return Err(anyhow::anyhow!(
                "Unexpected tensor rank for normalization: {}",
                r
            ))
        }
    };
    let norm = vector.sqr()?.sum_keepdim(dim)?.sqrt()?;
    Ok(vector.broadcast_div(&norm)?)
}

/// Stores a computed image semantic vector in the database cache within a transaction
fn store_semantic_vector_tx(
    tx: &rusqlite::Transaction,
    path: &str,
    semantic_vector: &Tensor,
) -> Result<()> {
    let vector_floats: Vec<f32> = semantic_vector.flatten_all()?.to_vec1()?;

    let id: i64 = tx.query_row(
        "INSERT INTO semantic_vector_path_mapping (path) VALUES (?)
         ON CONFLICT(path) DO UPDATE SET path=path
         RETURNING id",
        [path],
        |row| row.get(0),
    )?;

    tx.execute(
        "INSERT OR REPLACE INTO media_semantic_vectors (rowid, semantic_vector) VALUES (?, ?)",
        rusqlite::params![id, vector_floats.as_slice().as_bytes()],
    )?;

    Ok(())
}

/// Calculate frame sampling positions for video (evenly distributed, excluding endpoints)
fn calculate_frame_positions(duration: f64, count: usize) -> Vec<f64> {
    (1..=count)
        .map(|i| duration * (i as f64) / (count as f64 + 1.0))
        .collect()
}

/// Stores video semantic computation metadata within a transaction
fn store_video_metadata_tx(
    tx: &rusqlite::Transaction,
    path: &str,
    num_frames: usize,
    frame_times: &[f64],
    model_version: &str,
) -> Result<()> {
    let frame_times_json = serde_json::to_string(frame_times)?;

    tx.execute(
        "INSERT OR REPLACE INTO video_semantic_metadata (path, num_frames_sampled, frame_times, model_version)
         VALUES (?, ?, ?, ?)",
        rusqlite::params![path, num_frames as i64, frame_times_json, model_version],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_db_pool;
    use std::sync::OnceLock;

    type CachedModel = (Arc<RwLock<clip::ClipModel>>, Arc<Tokenizer>, Arc<Device>);

    static MODEL_CACHE: OnceLock<CachedModel> = OnceLock::new();

    fn get_cached_model() -> CachedModel {
        let (model, tokenizer, device) = MODEL_CACHE
            .get_or_init(|| {
                let device = Arc::new(Device::Cpu);
                // Ensure the path is correct relative to the project root where `cargo test` runs
                let (model, tokenizer) =
                    load_clip_model(&device, "./data").expect("Failed to load model");
                (Arc::new(RwLock::new(model)), Arc::new(tokenizer), device)
            })
            .clone();
        (model, tokenizer, device)
    }

    fn create_test_engine_cached(pool: crate::db::DbPool) -> SemanticSearchEngine {
        let (model, tokenizer, device) = get_cached_model();
        SemanticSearchEngine::new_with_model(model, tokenizer, device, pool)
    }

    #[test]
    fn test_duplicate_semantic_vector_skipped() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        let path = "test-data/cat.jpg";

        // Compute first time
        engine.compute_semantic_vector(path).unwrap();

        // Compute second time - should skip expensive inference
        let result = engine.compute_semantic_vector(path);
        assert!(result.is_ok(), "Second computation should succeed");
    }

    #[test]
    fn test_semantic_search_basic() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        // Index both images
        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();
        engine.compute_semantic_vector("test-data/car.jpg").unwrap();

        // Search for cat - should return cat.jpg first
        let results = engine.search("cat", 10, 0).unwrap();

        assert!(!results.is_empty(), "Search should return results");
        assert!(
            results[0].0.contains("cat"),
            "Top result should be cat.jpg, got: {}",
            results[0].0
        );
        assert!(
            results[0].1 >= 61.5,
            "Similarity score should be >= 61.5, got: {}",
            results[0].1
        );

        println!("Cat search results:");
        for (path, score) in &results {
            println!("  {}: {:.1}", path, score);
        }
    }

    #[test]
    fn test_semantic_search_concept_understanding() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();
        engine.compute_semantic_vector("test-data/car.jpg").unwrap();

        let results = engine.search("car", 10, 0).unwrap();

        assert!(!results.is_empty(), "Search should return results");
        assert!(
            results[0].0.contains("car"),
            "Car search should return car first, got: {}",
            results[0].0
        );
        assert!(
            results[0].1 >= 61.5,
            "Car score should be >= 61.5, got: {}",
            results[0].1
        );

        println!("Car search results:");
        for (path, score) in &results {
            println!("  {}: {:.1}", path, score);
        }
    }

    #[test]
    fn test_raw_image_embedding() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        let result = engine.compute_semantic_vector("test-data/IMG_9899.CR2");
        assert!(
            result.is_ok(),
            "RAW image should generate CLIP embedding: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_semantic_similarity_synonyms() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool);

        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();

        let queries = ["cat", "kitten", "feline"];

        for query in &queries {
            let results = engine.search(query, 10, 0).unwrap();
            assert!(
                !results.is_empty(),
                "Query '{}' should return results",
                query
            );
            assert!(
                results[0].0.contains("cat"),
                "Query '{}' should return cat.jpg",
                query
            );
            assert!(
                results[0].1 >= 61.5,
                "Query '{}' score should be >= 61.5, got {}",
                query,
                results[0].1
            );

            println!("Query '{}' score: {:.1}", query, results[0].1);
        }
    }

    #[test]
    fn test_search_empty_database() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool);

        let results = engine.search("cat", 10, 0).unwrap();

        assert!(
            results.is_empty(),
            "Search on empty database should return no results"
        );
    }

    #[test]
    fn test_minimum_similarity_threshold() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();
        engine.compute_semantic_vector("test-data/car.jpg").unwrap();

        let results = engine.search("cat", 10, 0).unwrap();

        for (path, score) in &results {
            assert!(
                *score >= 61.5,
                "All results should have similarity >= 61.5, got {} for {}",
                score,
                path
            );
        }

        println!("All {} results meet minimum threshold", results.len());
    }

    #[test]
    fn test_calculate_frame_positions() {
        // Test 30s video with 5 frames
        let positions = calculate_frame_positions(30.0, 5);
        assert_eq!(positions.len(), 5, "Should generate 5 frame positions");
        assert_eq!(positions, vec![5.0, 10.0, 15.0, 20.0, 25.0]);

        // Test 60s video with 5 frames
        let positions = calculate_frame_positions(60.0, 5);
        assert_eq!(positions.len(), 5);
        assert_eq!(positions, vec![10.0, 20.0, 30.0, 40.0, 50.0]);

        // Test short video (6s) with 5 frames
        let positions = calculate_frame_positions(6.0, 5);
        assert_eq!(positions.len(), 5);
        assert_eq!(positions, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[tokio::test]
    async fn test_video_embedding_computation() {
        let video_filename = "test_video.mp4";
        let video_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-data")
            .join(video_filename);

        if !video_path.exists() {
            eprintln!(
                "Skipping video embedding test: {} not found",
                video_filename
            );
            return;
        }

        let run_var = std::env::var("RUN_VIDEO_TESTS").unwrap_or_default();
        if !(run_var == "1" || run_var.eq_ignore_ascii_case("true")) {
            eprintln!("Skipping video embedding test: RUN_VIDEO_TESTS not set");
            return;
        }

        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        let video_path_str = video_path.to_string_lossy().to_string();
        let result = engine
            .compute_video_semantic_vector(&video_path_str, None)
            .await;

        assert!(
            result.is_ok(),
            "Video embedding computation should succeed: {:?}",
            result.err()
        );

        // Verify embedding was stored
        let conn = db_pool.get().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
                [&video_path_str],
                |row| row.get(0),
            )
            .unwrap();

        assert!(exists, "Video embedding should be stored in database");

        // Verify metadata was stored
        let metadata_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM video_semantic_metadata WHERE path = ?)",
                [&video_path_str],
                |row| row.get(0),
            )
            .unwrap();

        assert!(
            metadata_exists,
            "Video metadata should be stored in database"
        );
    }

    #[tokio::test]
    async fn test_video_search_integration() {
        let video_filename = "test_video.mp4";
        let video_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-data")
            .join(video_filename);

        if !video_path.exists() {
            eprintln!("Skipping video search test: {} not found", video_filename);
            return;
        }

        let run_var = std::env::var("RUN_VIDEO_TESTS").unwrap_or_default();
        if !(run_var == "1" || run_var.eq_ignore_ascii_case("true")) {
            eprintln!("Skipping video search test: RUN_VIDEO_TESTS not set");
            return;
        }

        let db_pool = create_test_db_pool().unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        // Index image and video
        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();
        let video_path_str = video_path.to_string_lossy().to_string();
        engine
            .compute_video_semantic_vector(&video_path_str, None)
            .await
            .unwrap();

        // Search should return both images and videos
        let results = engine.search("cat", 10, 0).unwrap();

        assert!(!results.is_empty(), "Search should return results");
        println!("Mixed search results:");
        for (path, score) in &results {
            println!("  {}: {:.1}", path, score);
        }
    }
}
