//! Semantic Image Search
//!
//! This module integrates OpenAI's CLIP model for semantic image search in TurboPix.
//! Image semantic vectors are cached in SQLite using sqlite-vec for efficient reuse.
//!
//! ## Architecture
//!
//! 1. **Model**: CLIP ViT-B/32 ONNX encoders from HuggingFace (512-dim semantic vectors)
//! 2. **Cache**: SQLite with sqlite-vec extension for vector storage
//! 3. **Pipeline**: Text query → encode → compare with cached image semantic vectors → rank
//!
//! The encoders live in [`crate::clip`]; this module owns the engine lifecycle,
//! the sqlite-vec queries and the video frame averaging.
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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use zerocopy::IntoBytes;

use crate::clip::{self, LoadedModel, ModelPaths, MODEL_VERSION};
use crate::db::DbPool;
use crate::ort_runtime::{rss_label, rss_mib};

// Model configuration
/// How long the CLIP sessions may sit unused before they are dropped. The two
/// ONNX graphs materialize ~600 MiB of weights, so a server nobody searches on
/// should not hold them. Deliberately a constant — there is no configuration
/// for it.
const CLIP_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
/// How often the idle monitor compares the last-use clock against the timeout.
const IDLE_MONITOR_TICK: Duration = Duration::from_secs(60);
/// Backoff after a failed resolve/load, so a broken model file or an
/// unreachable hub cannot be retried once per request.
const LOAD_RETRY_BACKOFF: Duration = Duration::from_secs(60);

fn retry_deadline() -> Instant {
    Instant::now() + LOAD_RETRY_BACKOFF
}

// Text encoding parameters live in `clip` with the tokenizer/model pins.

// Minimum similarity score threshold for search results (0.0-1.0 range)
// Normalized to 0-100 scale for user display (multiply by 100)
const MIN_SIMILARITY_SCORE: f32 = 0.615;

// Video frame sampling configuration
const VIDEO_FRAME_COUNT: usize = 3;

/// Monotonic counter shared by BOTH video-pipeline paths
/// (`compute_video_semantic_vector` and `encode_video_vector`) so their temp
/// frame directories can never collide. A per-function counter seeded at 0
/// lets concurrent invocations build the same `$TMPDIR/turbopix_<pid>_<n>`
/// path, and one `TempFrameDir` RAII guard would then delete the other's
/// frames mid-flight.
static TEMP_FRAME_COUNTER: AtomicU64 = AtomicU64::new(0);

/// RAII guard for a temporary frame-extraction directory: removes the
/// directory and everything in it on drop, so error paths cannot leak frames
/// under `$TMPDIR/turbopix_<pid>_<n>/`.
struct TempFrameDir(PathBuf);

impl TempFrameDir {
    fn new(unique_id: u64) -> Self {
        Self(std::env::temp_dir().join(format!("turbopix_{}_{}", std::process::id(), unique_id)))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFrameDir {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) => {}
            // The directory may never have been created (e.g. ffmpeg failed
            // before touching it) — that is not a leak, stay quiet.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                log::warn!(
                    "Failed to cleanup temp directory {}: {}",
                    self.0.display(),
                    e
                );
            }
        }
    }
}

pub struct VideoSemanticMeta {
    pub num_frames: usize,
    pub frame_times: Vec<f64>,
    pub model_version: String,
}

pub enum SemanticBatchItem {
    Image {
        path: String,
        vector: Vec<f32>,
    },
    Video {
        path: String,
        vector: Vec<f32>,
        meta: VideoSemanticMeta,
    },
}

/// Materialization state behind the single-flight lock: the resolved paths and
/// the backoff deadline after a failed attempt.
#[derive(Default)]
struct LoadState {
    paths: Option<ModelPaths>,
    retry_at: Option<Instant>,
}

/// Semantic search engine for image search.
///
/// The CLIP weights are materialized on first use and dropped after
/// [`CLIP_IDLE_TIMEOUT`] without one, so an idle server does not pay for a
/// feature nobody is using.
pub struct SemanticSearchEngine {
    loaded: RwLock<Option<LoadedModel>>,
    /// Held across the whole resolve+load section, so concurrent callers share
    /// one load instead of racing several ~600 MiB allocations.
    load_state: Mutex<LoadState>,
    data_path: String,
    pool: DbPool,
    /// Milliseconds since `started` at the beginning of the last model use.
    last_use_ms: AtomicU64,
    started: Instant,
}

#[async_trait]
pub trait SemanticSearch: Send + Sync {
    async fn search(&self, query: &str, limit: usize, offset: usize) -> Result<Vec<(String, f32)>>;
    async fn encode_image_vector(&self, image_path: &str) -> Result<(String, Vec<f32>)>;
    async fn encode_video_vector(
        &self,
        video_path: &str,
        frame_count: Option<usize>,
    ) -> Result<(String, Vec<f32>, VideoSemanticMeta)>;
}

/// Encodes sampled video frames into one normalized embedding.
fn encode_video_embedding(loaded: &LoadedModel, frames: &[PathBuf]) -> Result<Vec<f32>> {
    let batch = clip::encode_image_batch(loaded, frames)?;
    let mean = clip::average_vectors(&batch)?;
    clip::normalize_vector(mean)
}

impl SemanticSearchEngine {
    /// Resolves the model files (downloading them when the cache is cold) and
    /// leaves the weights unmaterialized.
    ///
    /// A failed resolve is not fatal: the server still serves photos, semantic
    /// search reports the failure per request, and a later attempt may succeed.
    pub fn new(db_pool: DbPool, data_path: &str) -> Self {
        let mut state = LoadState::default();
        match clip::resolve_model_paths(data_path) {
            Ok(paths) => {
                log::info!("CLIP model files ready: {}", paths.vision.display());
                state.paths = Some(paths);
            }
            Err(e) => {
                log::warn!(
                    "CLIP model files unavailable ({e:#}); semantic search stays disabled until a later attempt succeeds"
                );
                state.retry_at = Some(retry_deadline());
            }
        }

        Self {
            loaded: RwLock::new(None),
            load_state: Mutex::new(state),
            data_path: data_path.to_string(),
            pool: db_pool,
            last_use_ms: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_model(loaded: LoadedModel, pool: DbPool, data_path: &str) -> Self {
        Self {
            loaded: RwLock::new(Some(loaded)),
            load_state: Mutex::new(LoadState::default()),
            data_path: data_path.to_string(),
            pool,
            last_use_ms: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Stamps the last-use clock when a use begins, so the idle window counts
    /// from the start of the work. The work itself is kept safe by its own
    /// clone, not by this timestamp.
    fn touch(&self) {
        self.last_use_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Returns the materialized model, loading it single-flight when absent.
    async fn ensure_loaded(&self) -> Result<LoadedModel> {
        self.touch();
        if let Some(loaded) = self.loaded.read().await.clone() {
            return Ok(loaded);
        }

        let mut state = self.load_state.lock().await;
        // Another caller may have finished the load while we waited.
        if let Some(loaded) = self.loaded.read().await.clone() {
            return Ok(loaded);
        }
        if let Some(retry_at) = state.retry_at {
            let now = Instant::now();
            if now < retry_at {
                return Err(anyhow::anyhow!(
                    "CLIP model unavailable; retrying in {}s",
                    (retry_at - now).as_secs() + 1
                ));
            }
        }

        // The request path never resolves or downloads; the startup attempt and
        // the monitor's background retry own that.
        let Some(paths) = state.paths.clone() else {
            return Err(anyhow::anyhow!(
                "CLIP model files are unavailable; retrying in the background"
            ));
        };
        match tokio::task::spawn_blocking(move || clip::load_from_paths(&paths)).await {
            Ok(Ok(loaded)) => {
                state.retry_at = None;
                let returned = loaded.clone();
                *self.loaded.write().await = Some(loaded);
                Ok(returned)
            }
            Ok(Err(e)) => Err(self.arm_retry(state, e)),
            Err(e) => Err(self.arm_retry(state, anyhow::anyhow!("model load task failed: {e}"))),
        }
    }

    /// Arms the retry backoff and hands the error back.
    fn arm_retry(
        &self,
        mut state: tokio::sync::MutexGuard<'_, LoadState>,
        error: anyhow::Error,
    ) -> anyhow::Error {
        state.retry_at = Some(retry_deadline());
        log::warn!("CLIP model unavailable: {error:#}");
        error
    }

    /// Re-attempts the model-file resolve after a failed startup attempt. Runs
    /// from the idle monitor, so nothing on the request path touches the
    /// network.
    async fn retry_missing_model_files(&self) {
        if self.loaded.read().await.is_some() {
            return;
        }

        // Pre-check under the lock, then release it: a cold-cache resolve can
        // be a multi-minute download and must not block a waiting request.
        {
            let state = self.load_state.lock().await;
            if state.paths.is_some()
                || state
                    .retry_at
                    .is_some_and(|deadline| Instant::now() < deadline)
            {
                return;
            }
        }

        let data_path = self.data_path.clone();
        let resolved = match tokio::task::spawn_blocking(move || {
            clip::resolve_model_paths(&data_path)
        })
        .await
        {
            Ok(result) => result,
            Err(e) => Err(anyhow::anyhow!("resolve task failed: {e}")),
        };

        let mut state = self.load_state.lock().await;
        match resolved {
            Ok(paths) if state.paths.is_none() => {
                log::info!("CLIP model files ready: {}", paths.vision.display());
                state.paths = Some(paths);
                state.retry_at = None;
            }
            Ok(_) => {}
            Err(e) => {
                state.retry_at = Some(retry_deadline());
                log::warn!("CLIP model files still unavailable: {e:#}");
            }
        }
    }

    /// Materializes the model now instead of on first use.
    pub async fn warm_up(&self) -> Result<()> {
        self.ensure_loaded().await.map(|_| ())
    }

    /// Spawns the maintenance task: it drops the weights once they have been
    /// unused for [`CLIP_IDLE_TIMEOUT`], and retries the model-file resolve
    /// after a failed startup attempt so no request ever performs that
    /// (network) step. The task holds a weak reference, so it ends with the
    /// engine.
    pub fn start_idle_monitor(self: &Arc<Self>) {
        let engine = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(IDLE_MONITOR_TICK);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let Some(engine) = engine.upgrade() else {
                    break;
                };
                engine.retry_missing_model_files().await;
                engine.unload_if_idle();
            }
        });
    }

    /// Drops the weights when the idle window has passed. The memory itself is
    /// freed when the last in-flight encode releases its own clone, so the RSS
    /// figures below can legitimately match.
    fn unload_if_idle(&self) {
        // Readers only hold this lock for the clone, so `try_write` either gets
        // it immediately or skips to the next tick.
        let Ok(mut loaded) = self.loaded.try_write() else {
            return;
        };
        let now_ms = self.now_ms();
        let last_use_ms = self.last_use_ms.load(Ordering::Relaxed);
        if loaded.is_none() || !should_unload(now_ms, last_use_ms, CLIP_IDLE_TIMEOUT) {
            return;
        }

        let rss_before = rss_mib();
        // Take the model out and release the lock BEFORE it is dropped: the
        // last clone frees ~600 MiB and then trims the allocator (see
        // `LoadedModel::_trim`), and that work must not block a search that is
        // waiting to reload. The reading below still happens after the trim.
        let unloaded = loaded.take();
        drop(loaded);
        drop(unloaded);
        log::info!(
            "CLIP model unloaded after {}s idle (RSS {} MiB → {} MiB)",
            now_ms.saturating_sub(last_use_ms) / 1000,
            rss_label(rss_before),
            rss_label(rss_mib())
        );
    }

    /// Performs semantic search for a text query across all images using sqlite-vec KNN
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<(String, f32)>> {
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
            let loaded = self.ensure_loaded().await?;
            clip::encode_text(&loaded, query)?
        };
        log::debug!("Text encoding took: {:?}", encode_start.elapsed());

        // Convert semantic vector to bytes for sqlite-vec
        let vector_bytes = text_semantic_vector.as_slice().as_bytes().to_vec();

        // NOTE: the pinned sqlite-vec (v0.2.4-alpha) only uses its KNN index
        // when the query has a `MATCH` constraint on the vector column plus
        // `ORDER BY distance` — this query has neither, so the plan is a full
        // scan of the vector table (vec0Filter_fullscan) with a full sort.
        // KNN is deliberately not enabled: its k = LIMIT + OFFSET is capped
        // at 4096 and exceeding it is a hard SQL error, which deep semantic
        // pagination would trip.
        //
        // The score threshold is applied BEFORE LIMIT/OFFSET: filtering after
        // pagination truncates results — a page whose raw window holds fewer
        // than `limit` above-threshold rows would end pagination early even
        // though further valid matches exist. The CTE materializes the
        // distance ONCE per row (vec_distance_cosine is a 2048-byte blob
        // decode + dot product); evaluating it in both WHERE and ORDER BY
        // would double the per-row cost. similarity = 1 - distance / 2
        // (distance: 0 = identical, 2 = opposite).
        let db_start = std::time::Instant::now();
        let raw_results = sqlx::query_as::<_, (String, f32)>(
            "WITH scored AS (\n             SELECT ic.path, vec_distance_cosine(msv.semantic_vector, ?) AS dist\n             FROM media_semantic_vectors msv\n             JOIN semantic_vector_path_mapping ic ON msv.rowid = ic.id\n             )\n             SELECT path, 1.0 - (dist / 2.0) AS similarity\n             FROM scored\n             WHERE 1.0 - (dist / 2.0) >= ?\n             ORDER BY dist\n             LIMIT ? OFFSET ?",
        )
        .bind(&vector_bytes)
        .bind(MIN_SIMILARITY_SCORE)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;

        let results: Vec<(String, f32)> = raw_results
            .into_iter()
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
    pub async fn compute_semantic_vector(&self, image_path: &str) -> Result<()> {
        // OPTIMIZATION: Check existence BEFORE expensive computation
        // WAL mode allows concurrent reads without blocking writers
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
        )
        .bind(image_path)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(false);

        if exists {
            log::debug!("Semantic vector already cached for: {}", image_path);
            return Ok(());
        }

        // OPTIMIZATION: Compute expensive vector BEFORE opening the write transaction
        let semantic_vector = {
            let loaded = self.ensure_loaded().await?;
            clip::encode_image(&loaded, Path::new(image_path))?
        };

        // Use transaction to prevent race condition where multiple tasks compute same vector
        let mut tx = self.pool.begin().await?;

        // Double-check existence after expensive computation (race condition prevention)
        let still_missing: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
        )
        .bind(image_path)
        .fetch_one(&mut *tx)
        .await
        .unwrap_or(true);

        if still_missing {
            store_semantic_vector_tx(&mut tx, image_path, &semantic_vector).await?;
        } else {
            log::debug!(
                "Semantic vector was computed by another task for: {}",
                image_path
            );
        }

        // Mark the photo as indexed so it is not re-scanned on the next run
        // (mirrors batch_store_semantic_vectors).
        sqlx::query("UPDATE photos SET semantic_vector_indexed = 1 WHERE file_path = ?")
            .bind(image_path)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn encode_image_vector(&self, image_path: &str) -> Result<(String, Vec<f32>)> {
        let semantic_vector = {
            let loaded = self.ensure_loaded().await?;
            clip::encode_image(&loaded, Path::new(image_path))?
        };

        Ok((image_path.to_string(), semantic_vector))
    }

    /// Computes and caches semantic vector for a video by sampling frames
    pub async fn compute_video_semantic_vector(
        &self,
        video_path: &str,
        frame_count: Option<usize>,
    ) -> Result<()> {
        use crate::video_processor::{extract_frames_batch, extract_video_metadata};

        // Quick check without holding a transaction (to avoid holding lock across async)
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
        )
        .bind(video_path)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(false);

        if exists {
            log::debug!("Semantic vector already cached for video: {}", video_path);
            return Ok(());
        }

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

        // Create temp directory for frame extraction (shared counter — see
        // TEMP_FRAME_COUNTER)
        let unique_id = TEMP_FRAME_COUNTER.fetch_add(1, Ordering::SeqCst);
        // RAII guard: the directory (and any frames inside) is removed on
        // drop, so the error paths below can never leak it.
        let temp_dir = TempFrameDir::new(unique_id);

        // Step 1: Extract all frames in a single ffmpeg call (MUCH faster than parallel tasks)
        let extraction_start = std::time::Instant::now();
        let extracted_frames =
            extract_frames_batch(Path::new(video_path), &frame_times, temp_dir.path()).await?;
        log::debug!(
            "Extracted {} frames in {:?}",
            extracted_frames.len(),
            extraction_start.elapsed()
        );

        // Step 2: Encode frames to embeddings (batch inference)
        let normalized_embedding = {
            let loaded = self.ensure_loaded().await?;
            encode_video_embedding(&loaded, &extracted_frames)?
        };

        // Step 3: temp frames are cleaned up by TempFrameDir on drop.

        // Store embedding and metadata in a transaction (after all async work is done)
        let mut tx = self.pool.begin().await?;

        // Double-check it wasn't inserted by another concurrent task
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
        )
        .bind(video_path)
        .fetch_one(&mut *tx)
        .await
        .unwrap_or(false);

        if !exists {
            store_semantic_vector_tx(&mut tx, video_path, &normalized_embedding).await?;
            store_video_metadata_tx(
                &mut tx,
                video_path,
                frames_to_sample,
                &frame_times,
                MODEL_VERSION,
            )
            .await?;
        } else {
            log::debug!(
                "Video semantic vector was cached by another task during computation: {}",
                video_path
            );
        }

        // Mark the photo as indexed so it is not re-scanned on the next run
        // (mirrors batch_store_semantic_vectors).
        sqlx::query("UPDATE photos SET semantic_vector_indexed = 1 WHERE file_path = ?")
            .bind(video_path)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        log::info!(
            "Video semantic vector computed in {:?}: {}",
            start_time.elapsed(),
            video_path
        );

        Ok(())
    }

    pub async fn encode_video_vector(
        &self,
        video_path: &str,
        frame_count: Option<usize>,
    ) -> Result<(String, Vec<f32>, VideoSemanticMeta)> {
        use crate::video_processor::{extract_frames_batch, extract_video_metadata};

        log::info!("Computing semantic vector for video: {}", video_path);
        let start_time = std::time::Instant::now();
        let frames_to_sample = frame_count.unwrap_or(VIDEO_FRAME_COUNT);

        let metadata = extract_video_metadata(Path::new(video_path)).await?;
        let frame_times = calculate_frame_positions(metadata.duration, frames_to_sample);

        log::debug!(
            "Sampling {} frames at positions: {:?}",
            frame_times.len(),
            frame_times
        );

        // Shared counter — see TEMP_FRAME_COUNTER.
        let unique_id = TEMP_FRAME_COUNTER.fetch_add(1, Ordering::SeqCst);
        // RAII guard: the directory (and any frames inside) is removed on
        // drop, so the error paths below can never leak it.
        let temp_dir = TempFrameDir::new(unique_id);

        let extraction_start = std::time::Instant::now();
        let extracted_frames =
            extract_frames_batch(Path::new(video_path), &frame_times, temp_dir.path()).await?;
        log::debug!(
            "Extracted {} frames in {:?}",
            extracted_frames.len(),
            extraction_start.elapsed()
        );

        let normalized_embedding = {
            let loaded = self.ensure_loaded().await?;
            encode_video_embedding(&loaded, &extracted_frames)?
        };

        // Temp frames are cleaned up by TempFrameDir on drop.

        log::info!(
            "Video semantic vector computed in {:?}: {}",
            start_time.elapsed(),
            video_path
        );

        Ok((
            video_path.to_string(),
            normalized_embedding,
            VideoSemanticMeta {
                num_frames: frames_to_sample,
                frame_times,
                model_version: MODEL_VERSION.to_string(),
            },
        ))
    }
}

#[async_trait]
impl SemanticSearch for SemanticSearchEngine {
    async fn search(&self, query: &str, limit: usize, offset: usize) -> Result<Vec<(String, f32)>> {
        self.search(query, limit, offset).await
    }

    async fn encode_image_vector(&self, image_path: &str) -> Result<(String, Vec<f32>)> {
        self.encode_image_vector(image_path).await
    }

    async fn encode_video_vector(
        &self,
        video_path: &str,
        frame_count: Option<usize>,
    ) -> Result<(String, Vec<f32>, VideoSemanticMeta)> {
        self.encode_video_vector(video_path, frame_count).await
    }
}

#[cfg(test)]
pub struct NoopSemanticSearch;

#[cfg(test)]
#[async_trait]
impl SemanticSearch for NoopSemanticSearch {
    async fn search(
        &self,
        _query: &str,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<(String, f32)>> {
        Ok(vec![])
    }

    async fn encode_image_vector(&self, _image_path: &str) -> Result<(String, Vec<f32>)> {
        Err(anyhow::anyhow!(
            "NoopSemanticSearch: semantic search not available"
        ))
    }

    async fn encode_video_vector(
        &self,
        _video_path: &str,
        _frame_count: Option<usize>,
    ) -> Result<(String, Vec<f32>, VideoSemanticMeta)> {
        Err(anyhow::anyhow!(
            "NoopSemanticSearch: semantic search not available"
        ))
    }
}

pub async fn batch_store_semantic_vectors(
    pool: &DbPool,
    items: Vec<SemanticBatchItem>,
) -> Result<(usize, usize)> {
    let mut tx = pool.begin().await?;
    let mut success_count = 0;
    let mut error_count = 0;

    for item in items {
        match item {
            SemanticBatchItem::Image { path, vector } => {
                if let Err(e) = store_semantic_vector_tx(&mut tx, &path, &vector).await {
                    log::error!("Failed to store semantic vector {}: {}", path, e);
                    error_count += 1;
                    continue;
                }

                if let Err(e) =
                    sqlx::query("UPDATE photos SET semantic_vector_indexed = 1 WHERE file_path = ?")
                        .bind(&path)
                        .execute(&mut *tx)
                        .await
                {
                    log::error!("Failed to mark photo as indexed {}: {}", path, e);
                    error_count += 1;
                    continue;
                }

                success_count += 1;
            }
            SemanticBatchItem::Video { path, vector, meta } => {
                if let Err(e) = store_semantic_vector_tx(&mut tx, &path, &vector).await {
                    log::error!("Failed to store semantic vector {}: {}", path, e);
                    error_count += 1;
                    continue;
                }

                if let Err(e) = store_video_metadata_tx(
                    &mut tx,
                    &path,
                    meta.num_frames,
                    &meta.frame_times,
                    &meta.model_version,
                )
                .await
                {
                    log::error!("Failed to store video metadata {}: {}", path, e);
                    error_count += 1;
                    continue;
                }

                if let Err(e) =
                    sqlx::query("UPDATE photos SET semantic_vector_indexed = 1 WHERE file_path = ?")
                        .bind(&path)
                        .execute(&mut *tx)
                        .await
                {
                    log::error!("Failed to mark photo as indexed {}: {}", path, e);
                    error_count += 1;
                    continue;
                }

                success_count += 1;
            }
        }
    }

    tx.commit().await?;
    Ok((success_count, error_count))
}

/// Downloads CLIP model files to the cache directory
pub fn download_models(data_path: &str) -> Result<()> {
    log::info!("Downloading CLIP model to cache...");
    let paths = clip::resolve_model_paths(data_path)?;
    log::info!("Vision encoder downloaded: {}", paths.vision.display());
    log::info!("Text encoder downloaded: {}", paths.text.display());
    log::info!("Tokenizer downloaded: {}", paths.tokenizer.display());
    Ok(())
}

/// Whether the model has been unused for at least `idle`. A clock that reads
/// earlier than its own last-use stamp counts as "just used", never as idle.
fn should_unload(now_ms: u64, last_use_ms: u64, idle: Duration) -> bool {
    now_ms.saturating_sub(last_use_ms) >= idle.as_millis() as u64
}

/// Stores a computed image semantic vector in the database cache within a transaction
async fn store_semantic_vector_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    path: &str,
    semantic_vector: &[f32],
) -> Result<()> {
    if semantic_vector.len() != clip::EMBEDDING_DIM {
        anyhow::bail!(
            "Unexpected embedding length for {}: expected {}, got {}",
            path,
            clip::EMBEDDING_DIM,
            semantic_vector.len()
        );
    }

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO semantic_vector_path_mapping (path) VALUES (?)
         ON CONFLICT(path) DO UPDATE SET path=path
         RETURNING id",
    )
    .bind(path)
    .fetch_one(&mut **tx)
    .await?;

    // vec0 virtual tables do not honor INSERT OR REPLACE conflict handling:
    // a row left by an interrupted run (vector stored, indexed flag not yet
    // committed) makes the insert fail with a UNIQUE constraint and the retry
    // never converges. Delete the old row explicitly first.
    sqlx::query("DELETE FROM media_semantic_vectors WHERE rowid = ?")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO media_semantic_vectors (rowid, semantic_vector) VALUES (?, ?)")
        .bind(id)
        .bind(semantic_vector.as_bytes())
        .execute(&mut **tx)
        .await?;

    Ok(())
}

/// Calculate frame sampling positions for video (evenly distributed, excluding endpoints)
fn calculate_frame_positions(duration: f64, count: usize) -> Vec<f64> {
    (1..=count)
        .map(|i| duration * (i as f64) / (count as f64 + 1.0))
        .collect()
}

/// Stores video semantic computation metadata within a transaction
async fn store_video_metadata_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    path: &str,
    num_frames: usize,
    frame_times: &[f64],
    model_version: &str,
) -> Result<()> {
    let frame_times_json = serde_json::to_string(frame_times)?;

    sqlx::query(
        "INSERT OR REPLACE INTO video_semantic_metadata (path, num_frames_sampled, frame_times, model_version)
         VALUES (?, ?, ?, ?)",
    )
    .bind(path)
    .bind(num_frames as i64)
    .bind(frame_times_json)
    .bind(model_version)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_test_db_pool;
    use crate::db::Photo;
    use crate::video_processor::tests::acquire_test_env_lock;
    use chrono::Utc;
    use serde_json::json;

    fn create_test_engine_cached(pool: crate::db::DbPool) -> SemanticSearchEngine {
        let loaded = crate::clip::cached_test_model();
        SemanticSearchEngine::new_with_model(loaded, pool, "./data")
    }

    #[test]
    fn unloads_only_after_the_full_idle_window() {
        let idle = Duration::from_secs(600);
        assert!(!should_unload(599_999, 0, idle));
        assert!(should_unload(600_000, 0, idle));
    }

    #[test]
    fn idle_window_restarts_from_the_last_use() {
        let idle = Duration::from_secs(600);
        // Last use at 120 s ⇒ the window expires at 720 s.
        assert!(!should_unload(719_999, 120_000, idle));
        assert!(should_unload(720_000, 120_000, idle));
        // A clock that reads earlier than the last use is not "idle".
        assert!(!should_unload(0, 120_000, idle));
    }

    fn create_test_photo(path: &str) -> Photo {
        let now = Utc::now();
        let hash_seed = path.bytes().fold(0_u64, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(byte as u64)
        });

        Photo {
            hash_sha256: format!("{hash_seed:064x}"),
            file_path: path.to_string(),
            filename: std::path::Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_string(),
            file_size: 123,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            taken_latitude: None,
            taken_longitude: None,
            width: Some(100),
            height: Some(100),
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: json!({}),
            date_modified: now,
            date_indexed: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    fn create_dummy_vector(value: f32) -> Vec<f32> {
        vec![value; clip::EMBEDDING_DIM]
    }

    #[tokio::test]
    async fn test_batch_store_semantic_vectors_writes_all_items() {
        // GIVEN a database with photos waiting for semantic indexing
        let db_pool = create_test_db_pool().await.unwrap();
        for path in [
            "test-data/batch-one.jpg",
            "test-data/batch-two.jpg",
            "test-data/batch-three.jpg",
        ] {
            create_test_photo(path).create(&db_pool).await.unwrap();
        }

        let items = vec![
            SemanticBatchItem::Image {
                path: "test-data/batch-one.jpg".to_string(),
                vector: create_dummy_vector(0.1),
            },
            SemanticBatchItem::Image {
                path: "test-data/batch-two.jpg".to_string(),
                vector: create_dummy_vector(0.2),
            },
            SemanticBatchItem::Image {
                path: "test-data/batch-three.jpg".to_string(),
                vector: create_dummy_vector(0.3),
            },
        ];

        // WHEN the batch store writes all vectors in one transaction
        let result = batch_store_semantic_vectors(&db_pool, items).await.unwrap();

        // THEN all semantic rows and indexed flags are persisted
        let mapping_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM semantic_vector_path_mapping")
                .fetch_one(&db_pool)
                .await
                .unwrap();
        assert_eq!(mapping_count, 3);

        let vector_count: i64 = sqlx::query_scalar("SELECT count(*) FROM media_semantic_vectors")
            .fetch_one(&db_pool)
            .await
            .unwrap();
        assert_eq!(vector_count, 3);

        let indexed_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM photos WHERE semantic_vector_indexed = 1")
                .fetch_one(&db_pool)
                .await
                .unwrap();
        assert_eq!(indexed_count, 3);

        assert_eq!(result, (3, 0));
    }

    #[tokio::test]
    async fn test_batch_store_replaces_vector_left_by_interrupted_scan() {
        // GIVEN a previous run stored the vector but was killed before the
        // photo's indexed flag was committed
        let db_pool = create_test_db_pool().await.unwrap();
        let path = "test-data/interrupted.jpg";
        create_test_photo(path).create(&db_pool).await.unwrap();
        let first = vec![SemanticBatchItem::Image {
            path: path.to_string(),
            vector: create_dummy_vector(0.1),
        }];
        assert_eq!(
            batch_store_semantic_vectors(&db_pool, first).await.unwrap(),
            (1, 0)
        );
        sqlx::query("UPDATE photos SET semantic_vector_indexed = 0 WHERE file_path = ?")
            .bind(path)
            .execute(&db_pool)
            .await
            .unwrap();

        // WHEN the retry computes and stores the same path again
        let retry = vec![SemanticBatchItem::Image {
            path: path.to_string(),
            vector: create_dummy_vector(0.2),
        }];
        let result = batch_store_semantic_vectors(&db_pool, retry).await.unwrap();

        // THEN the vec0 row is replaced instead of failing on its primary key
        assert_eq!(result, (1, 0), "a stored vector must be replaceable");
        let vector_count: i64 = sqlx::query_scalar("SELECT count(*) FROM media_semantic_vectors")
            .fetch_one(&db_pool)
            .await
            .unwrap();
        assert_eq!(vector_count, 1);
        let indexed: i64 =
            sqlx::query_scalar("SELECT semantic_vector_indexed FROM photos WHERE file_path = ?")
                .bind(path)
                .fetch_one(&db_pool)
                .await
                .unwrap();
        assert_eq!(indexed, 1);
    }

    #[tokio::test]
    async fn test_batch_store_empty_batch_succeeds() {
        // GIVEN an empty semantic batch
        let db_pool = create_test_db_pool().await.unwrap();

        // WHEN the batch store runs
        let result = batch_store_semantic_vectors(&db_pool, vec![])
            .await
            .unwrap();

        // THEN it succeeds without writing semantic rows
        assert_eq!(result, (0, 0));

        let mapping_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM semantic_vector_path_mapping")
                .fetch_one(&db_pool)
                .await
                .unwrap();
        assert_eq!(mapping_count, 0);

        let vector_count: i64 = sqlx::query_scalar("SELECT count(*) FROM media_semantic_vectors")
            .fetch_one(&db_pool)
            .await
            .unwrap();
        assert_eq!(vector_count, 0);
    }

    #[tokio::test]
    async fn test_duplicate_semantic_vector_skipped() {
        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        let path = "test-data/cat.jpg";

        // Compute first time
        engine.compute_semantic_vector(path).await.unwrap();

        // Compute second time - should skip expensive inference
        let result = engine.compute_semantic_vector(path).await;
        assert!(result.is_ok(), "Second computation should succeed");
    }

    #[tokio::test]
    async fn test_semantic_search_basic() {
        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        // Index both images
        engine
            .compute_semantic_vector("test-data/cat.jpg")
            .await
            .unwrap();
        engine
            .compute_semantic_vector("test-data/car.jpg")
            .await
            .unwrap();

        // Search for cat - should return cat.jpg first
        let results = engine.search("cat", 10, 0).await.unwrap();

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

    #[tokio::test]
    async fn test_semantic_search_concept_understanding() {
        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        engine
            .compute_semantic_vector("test-data/cat.jpg")
            .await
            .unwrap();
        engine
            .compute_semantic_vector("test-data/car.jpg")
            .await
            .unwrap();

        let results = engine.search("car", 10, 0).await.unwrap();

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

    #[tokio::test]
    async fn test_raw_image_embedding() {
        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        let result = engine
            .compute_semantic_vector("test-data/IMG_9899.CR2")
            .await;
        assert!(
            result.is_ok(),
            "RAW image should generate CLIP embedding: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_semantic_similarity_synonyms() {
        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool);

        engine
            .compute_semantic_vector("test-data/cat.jpg")
            .await
            .unwrap();

        let queries = ["cat", "kitten", "feline"];

        for query in &queries {
            let results = engine.search(query, 10, 0).await.unwrap();
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

    #[tokio::test]
    async fn test_search_empty_database() {
        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool);

        let results = engine.search("cat", 10, 0).await.unwrap();

        assert!(
            results.is_empty(),
            "Search on empty database should return no results"
        );
    }

    #[tokio::test]
    async fn test_minimum_similarity_threshold() {
        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        engine
            .compute_semantic_vector("test-data/cat.jpg")
            .await
            .unwrap();
        engine
            .compute_semantic_vector("test-data/car.jpg")
            .await
            .unwrap();

        let results = engine.search("cat", 10, 0).await.unwrap();

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

        if !crate::video_processor::tests::video_tests_enabled() {
            eprintln!("Skipping video embedding test: RUN_VIDEO_TESTS not set");
            return;
        }
        let _env_lock = acquire_test_env_lock();

        let db_pool = create_test_db_pool().await.unwrap();
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
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM semantic_vector_path_mapping WHERE path = ?)",
        )
        .bind(&video_path_str)
        .fetch_one(&db_pool)
        .await
        .unwrap();

        assert!(exists, "Video embedding should be stored in database");

        // Verify metadata was stored
        let metadata_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM video_semantic_metadata WHERE path = ?)",
        )
        .bind(&video_path_str)
        .fetch_one(&db_pool)
        .await
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

        if !crate::video_processor::tests::video_tests_enabled() {
            eprintln!("Skipping video search test: RUN_VIDEO_TESTS not set");
            return;
        }
        let _env_lock = acquire_test_env_lock();

        let db_pool = create_test_db_pool().await.unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        // Index image and video
        engine
            .compute_semantic_vector("test-data/cat.jpg")
            .await
            .unwrap();
        engine
            .compute_semantic_vector("test-data/car.jpg")
            .await
            .unwrap();
        let video_path_str = video_path.to_string_lossy().to_string();
        engine
            .compute_video_semantic_vector(&video_path_str, None)
            .await
            .unwrap();

        // Search should return both images and videos
        let results = engine.search("cat", 10, 0).await.unwrap();

        assert!(!results.is_empty(), "Search should return results");
        println!("Mixed search results:");
        for (path, score) in &results {
            println!("  {}: {:.1}", path, score);
        }
    }

    #[test]
    fn test_temp_frame_counter_shared_across_video_paths() {
        // Regression: each video path used to declare its own `static COUNTER`
        // seeded at 0, so concurrent invocations of compute_video_semantic_vector
        // and encode_video_vector could build the same
        // `$TMPDIR/turbopix_<pid>_<n>` path and one TempFrameDir guard would
        // delete the other's frames mid-flight. Both paths must now draw from
        // the one module-level counter, so consecutive allocations never
        // collide.
        let id_a = TEMP_FRAME_COUNTER.fetch_add(1, Ordering::SeqCst);
        let id_b = TEMP_FRAME_COUNTER.fetch_add(1, Ordering::SeqCst);
        assert_ne!(id_a, id_b, "shared counter must issue distinct ids");
        assert_ne!(
            TempFrameDir::new(id_a).path(),
            TempFrameDir::new(id_b).path(),
            "distinct ids must yield distinct temp directories"
        );
    }

    #[test]
    fn test_temp_frame_dir_cleans_up_on_drop() {
        // GIVEN: a temp frame directory guard with a frame file inside
        let temp_dir = TempFrameDir::new(0xdead_beef);
        let path = temp_dir.path().to_path_buf();
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("frame_0.jpg"), b"jpeg").unwrap();

        // WHEN: the guard goes out of scope (as it would on both success and
        // error paths of video embedding)
        drop(temp_dir);

        // THEN: the directory and its contents are removed
        assert!(
            !path.exists(),
            "temp dir should be removed on drop, even with frames inside"
        );
    }

    #[tokio::test]
    async fn test_compute_semantic_vector_marks_photo_indexed() {
        // GIVEN: a photo row waiting for semantic indexing
        let db_pool = create_test_db_pool().await.unwrap();
        let path = "test-data/cat.jpg";
        create_test_photo(path).create(&db_pool).await.unwrap();
        let engine = create_test_engine_cached(db_pool.clone());

        // WHEN: the single-image compute path runs (the production caller
        // path, as opposed to the batch path)
        engine.compute_semantic_vector(path).await.unwrap();

        // THEN: the photo is no longer a perpetual re-scan candidate
        let indexed: Option<bool> =
            sqlx::query_scalar("SELECT semantic_vector_indexed FROM photos WHERE file_path = ?")
                .bind(path)
                .fetch_one(&db_pool)
                .await
                .unwrap();
        assert_eq!(
            indexed,
            Some(true),
            "semantic_vector_indexed should be set to 1 after computing"
        );
    }
}
