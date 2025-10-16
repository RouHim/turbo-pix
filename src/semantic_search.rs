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
use rusqlite::Connection;
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

    /// Performs semantic search for a text query across all images using sqlite-vec KNN
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>> {
        let start_time = std::time::Instant::now();
        log::info!("Semantic search for: '{}'", query);

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
            "SELECT ic.path, 1.0 - (vec_distance_cosine(isv.semantic_vector, ?) / 2.0) as similarity
             FROM image_semantic_vectors isv
             JOIN semantic_vector_path_mapping ic ON isv.rowid = ic.id
             ORDER BY vec_distance_cosine(isv.semantic_vector, ?)
             LIMIT ?",
        )?;

        let results: Vec<(String, f32)> = stmt
            .query_map(
                rusqlite::params![&vector_bytes, &vector_bytes, limit as i64],
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
        // Check if semantic vector already exists to avoid recomputation
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

        let model_read = self
            .model
            .read()
            .map_err(|e| anyhow::anyhow!("Failed to acquire model lock: {}", e))?;
        let semantic_vector = encode_image(&model_read, image_path, &self.device)?;
        drop(model_read);

        store_semantic_vector(&conn, image_path, &semantic_vector)?;

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

    let resized = img.resize_exact(
        CLIP_IMAGE_SIZE,
        CLIP_IMAGE_SIZE,
        image::imageops::FilterType::Triangle,
    );

    let img_rgb = resized.to_rgb8();

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

/// Computes L2 normalization of a semantic vector
fn normalize_vector(vector: &Tensor) -> Result<Tensor> {
    let norm = vector.sqr()?.sum_keepdim(1)?.sqrt()?;
    Ok(vector.broadcast_div(&norm)?)
}

/// Stores a computed image semantic vector in the database cache
fn store_semantic_vector(db: &Connection, path: &str, semantic_vector: &Tensor) -> Result<()> {
    let vector_floats: Vec<f32> = semantic_vector.flatten_all()?.to_vec1()?;

    let id: i64 = db.query_row(
        "INSERT INTO semantic_vector_path_mapping (path) VALUES (?)
         ON CONFLICT(path) DO UPDATE SET path=path
         RETURNING id",
        [path],
        |row| row.get(0),
    )?;

    db.execute(
        "INSERT OR REPLACE INTO image_semantic_vectors (rowid, semantic_vector) VALUES (?, ?)",
        rusqlite::params![id, vector_floats.as_slice().as_bytes()],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_db_pool;

    #[test]
    fn test_duplicate_semantic_vector_skipped() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = SemanticSearchEngine::new(db_pool.clone(), "./data").unwrap();

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
        let engine = SemanticSearchEngine::new(db_pool.clone(), "./data").unwrap();

        // Index both images
        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();
        engine.compute_semantic_vector("test-data/car.jpg").unwrap();

        // Search for cat - should return cat.jpg first
        let results = engine.search("cat", 10).unwrap();

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
        let engine = SemanticSearchEngine::new(db_pool.clone(), "./data").unwrap();

        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();
        engine.compute_semantic_vector("test-data/car.jpg").unwrap();

        let results = engine.search("car", 10).unwrap();

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
        let engine = SemanticSearchEngine::new(db_pool.clone(), "./data").unwrap();

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
        let engine = SemanticSearchEngine::new(db_pool, "./data").unwrap();

        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();

        let queries = ["cat", "kitten", "feline"];

        for query in &queries {
            let results = engine.search(query, 10).unwrap();
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
        let engine = SemanticSearchEngine::new(db_pool, "./data").unwrap();

        let results = engine.search("cat", 10).unwrap();

        assert!(
            results.is_empty(),
            "Search on empty database should return no results"
        );
    }

    #[test]
    fn test_minimum_similarity_threshold() {
        let db_pool = create_test_db_pool().unwrap();
        let engine = SemanticSearchEngine::new(db_pool.clone(), "./data").unwrap();

        engine.compute_semantic_vector("test-data/cat.jpg").unwrap();
        engine.compute_semantic_vector("test-data/car.jpg").unwrap();

        let results = engine.search("cat", 10).unwrap();

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
}
