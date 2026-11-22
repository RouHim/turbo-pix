use clokwerk::{Job, Scheduler, TimeUnits};
use log::{error, info, warn};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::cache_manager::CacheManager;
use crate::collage_generator;
use crate::db::DbPool;
use crate::face_recognition::FaceRecognition;
use crate::geo_location::NominatimClient;
use crate::housekeeping_manager;
use crate::indexer::PhotoProcessor;
use crate::semantic_search::SemanticSearch;
#[cfg(test)]
use crate::semantic_search::SemanticSearchEngine;

/// Batch size for database writes (I/O-bound operations)
/// Larger batches (250) reduce transaction overhead and improve throughput.
/// Higher than SEMANTIC_BATCH_SIZE since database writes are I/O-bound
/// and benefit from larger transactions to amortize COMMIT costs.
const DB_WRITE_BATCH_SIZE: usize = 250;

#[derive(Debug, Clone, Default)]
pub struct PhaseSnapshot {
    pub processed: u64,
    pub total: u64,
    pub errors: u64,
    pub current_item: Option<String>,
}

/// Per-phase counters for indexing progress tracking.
/// All fields are atomic/mutex for thread-safe concurrent access.
pub struct PhaseCounters {
    processed: AtomicU64,
    total: AtomicU64,
    errors: AtomicU64,
    current_item: std::sync::Mutex<Option<String>>,
}

impl PhaseCounters {
    fn new() -> Self {
        Self {
            processed: AtomicU64::new(0),
            total: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            current_item: std::sync::Mutex::new(None),
        }
    }

    pub fn reset(&self) {
        self.processed.store(0, Ordering::SeqCst);
        self.total.store(0, Ordering::SeqCst);
        self.errors.store(0, Ordering::SeqCst);
        *self.current_item.lock().unwrap() = None;
    }

    /// Set total monotonically — never decreases within a run.
    /// Uses `fetch_max` so concurrent or repeated calls cannot lower the total.
    pub fn set_total(&self, new_total: u64) {
        self.total.fetch_max(new_total, Ordering::SeqCst);
    }

    pub fn set_processed(&self, count: u64) {
        self.processed.store(count, Ordering::SeqCst);
    }

    pub fn add_processed(&self, count: u64) {
        self.processed.fetch_add(count, Ordering::SeqCst);
    }

    pub fn add_errors(&self, count: u64) {
        self.errors.fetch_add(count, Ordering::SeqCst);
    }

    pub fn set_current_item(&self, item: Option<String>) {
        *self.current_item.lock().unwrap() = item;
    }

    pub fn snapshot(&self) -> PhaseSnapshot {
        PhaseSnapshot {
            processed: self.processed.load(Ordering::SeqCst),
            total: self.total.load(Ordering::SeqCst),
            errors: self.errors.load(Ordering::SeqCst),
            current_item: self.current_item.lock().unwrap().clone(),
        }
    }
}

pub struct IndexingPhases {
    pub discovering: PhaseCounters,
    pub metadata: PhaseCounters,
    pub geo_resolution: PhaseCounters,
    pub semantic_vectors: PhaseCounters,
    pub faces: PhaseCounters,
    pub collages: PhaseCounters,
    pub housekeeping: PhaseCounters,
}

impl Default for IndexingPhases {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexingPhases {
    pub fn new() -> Self {
        Self {
            discovering: PhaseCounters::new(),
            metadata: PhaseCounters::new(),
            geo_resolution: PhaseCounters::new(),
            semantic_vectors: PhaseCounters::new(),
            faces: PhaseCounters::new(),
            collages: PhaseCounters::new(),
            housekeeping: PhaseCounters::new(),
        }
    }

    pub fn reset_all(&self) {
        self.discovering.reset();
        self.metadata.reset();
        self.geo_resolution.reset();
        self.semantic_vectors.reset();
        self.faces.reset();
        self.collages.reset();
        self.housekeeping.reset();
    }

    pub fn get(&self, phase_id: &str) -> Option<&PhaseCounters> {
        match phase_id {
            "discovering" => Some(&self.discovering),
            "metadata" => Some(&self.metadata),
            "geo_resolution" => Some(&self.geo_resolution),
            "semantic_vectors" => Some(&self.semantic_vectors),
            "faces" => Some(&self.faces),
            "collages" => Some(&self.collages),
            "housekeeping" => Some(&self.housekeeping),
            _ => None,
        }
    }
}

/// Indexing status shared across threads
#[derive(Clone)]
pub struct IndexingStatus {
    pub is_indexing: Arc<AtomicBool>,
    pub is_complete: Arc<AtomicBool>,
    /// Whether phase 1 could read every configured root. A partial scan still
    /// runs the later phases over what it found, but it must not mark the
    /// index complete (the next scan may heal the missing rows).
    pub scan_complete: Arc<AtomicBool>,
    pub current_phase: Arc<Mutex<String>>,
    pub phases: Arc<IndexingPhases>,
    pub started_at: Arc<Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
}

impl Default for IndexingStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexingStatus {
    pub fn new() -> Self {
        Self {
            is_indexing: Arc::new(AtomicBool::new(false)),
            is_complete: Arc::new(AtomicBool::new(false)),
            scan_complete: Arc::new(AtomicBool::new(true)),
            current_phase: Arc::new(Mutex::new(String::from("idle"))),
            phases: Arc::new(IndexingPhases::new()),
            started_at: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start_indexing(&self) {
        self.is_indexing.store(true, Ordering::SeqCst);
        self.scan_complete.store(true, Ordering::SeqCst);
        *self.started_at.lock().await = Some(chrono::Utc::now());
        self.phases.reset_all();
        *self.current_phase.lock().await = String::from("discovering");
    }

    pub async fn stop_indexing(&self) {
        self.is_indexing.store(false, Ordering::SeqCst);
        *self.current_phase.lock().await = String::from("idle");
        *self.started_at.lock().await = None;
    }

    pub fn mark_complete(&self) {
        self.is_complete.store(true, Ordering::SeqCst);
    }

    pub fn mark_incomplete(&self) {
        self.is_complete.store(false, Ordering::SeqCst);
    }

    pub async fn set_phase(&self, phase: &str) {
        *self.current_phase.lock().await = String::from(phase);
    }
}

/// Outcome of a manual scan request. `AlreadyRunning` means the request
/// attached to the in-flight cycle instead of spawning a second one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTrigger {
    Started,
    AlreadyRunning,
}

/// The two indexing pipelines the scheduler calls. Grouping them keeps the
/// constructor's wiring arguments below clippy's limit without hiding a
/// dependency behind a generic bag.
pub struct Pipelines {
    pub semantic_search: Arc<dyn SemanticSearch>,
    pub face_recognition: Arc<dyn FaceRecognition>,
}

#[derive(Clone)]
pub struct PhotoScheduler {
    photo_paths: Vec<PathBuf>,
    db_pool: DbPool,
    cache_manager: CacheManager,
    semantic_search: Arc<dyn SemanticSearch>,
    face_recognition: Arc<dyn FaceRecognition>,
    data_path: PathBuf,
    locale: String,
    nominatim_url: String,
    rescan_lock: Arc<Semaphore>,
    pub status: IndexingStatus,
}

impl PhotoScheduler {
    /// Every dependency the full rescan needs. The constructor is the single
    /// wiring point for the call graph.
    pub fn new(
        photo_paths: Vec<PathBuf>,
        db_pool: DbPool,
        cache_manager: CacheManager,
        pipelines: Pipelines,
        data_path: PathBuf,
        locale: String,
        nominatim_url: String,
    ) -> Self {
        let Pipelines {
            semantic_search,
            face_recognition,
        } = pipelines;
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
            face_recognition,
            data_path,
            locale,
            nominatim_url,
            rescan_lock: Arc::new(Semaphore::new(1)),
            status: IndexingStatus::new(),
        }
    }

    /// Helper: Batch write photos to database with transaction support
    ///
    /// Writes photos in batches to minimize database lock duration.
    /// Uses sqlx transactions for async database operations.
    ///
    /// Returns (successful_count, error_count)
    async fn batch_write_photos(
        db_pool: &DbPool,
        photos: Vec<crate::db::Photo>,
        status: &IndexingStatus,
    ) -> (usize, usize) {
        let mut indexed_count = 0;
        let mut error_count = 0;
        let total_count = photos.len();

        status.phases.metadata.set_total(total_count as u64);

        for (batch_idx, batch) in photos.chunks(DB_WRITE_BATCH_SIZE).enumerate() {
            // Begin transaction
            let mut tx = match db_pool.begin().await {
                Ok(tx) => tx,
                Err(e) => {
                    error!("Failed to begin transaction: {}", e);
                    error_count += batch.len();
                    continue;
                }
            };

            let mut batch_success = 0;

            for photo in batch {
                match photo.create_or_update_with_transaction(&mut tx).await {
                    Ok(_) => batch_success += 1,
                    Err(e) => {
                        error!("Failed to save photo {}: {}", photo.file_path, e);
                        error_count += 1;
                    }
                }
            }

            // Commit transaction (partial success is ok - errors already counted)
            match tx.commit().await {
                Ok(_) => {
                    indexed_count += batch_success;
                    status.phases.metadata.set_processed(indexed_count as u64);
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

        status.phases.metadata.add_errors(error_count as u64);

        (indexed_count, error_count)
    }

    pub fn start(&self) -> JoinHandle<()> {
        let mut scheduler = Scheduler::new();
        let cycle = self.clone();

        // Full rescan and cleanup at midnight
        scheduler.every(1.day()).at("00:00").run(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Err(e) = cycle.run_cycle().await {
                    error!("Scheduled rescan failed: {}", e);
                }
            });
        });

        // Database vacuum at 00:05 (after rescan completes)
        let db_pool_vacuum = self.db_pool.clone();
        scheduler.every(1.day()).at("00:05").run(move || {
            info!("Starting scheduled database vacuum");

            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match crate::db::vacuum_database(&db_pool_vacuum).await {
                    Ok(_) => info!("Database vacuum completed successfully"),
                    // A locked database (live API traffic) is the expected
                    // skip case — VACUUM retries on the next night.
                    Err(e) => warn!("Scheduled vacuum skipped: {}", e),
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

    async fn run_geo_resolution_phase(
        db_pool: &DbPool,
        nominatim_url: &str,
        status: &IndexingStatus,
    ) {
        let photos = match crate::db::get_photos_needing_geo_resolution(db_pool).await {
            Ok(photos) => photos,
            Err(e) => {
                error!("Failed to query photos needing geo resolution: {}", e);
                return;
            }
        };

        if photos.is_empty() {
            info!("No photos need geo resolution");
            return;
        }

        let total = photos.len();
        info!("Geo resolution: {} photos to resolve", total);
        status.phases.geo_resolution.set_total(total as u64);

        let mut client = NominatimClient::new(nominatim_url);
        if let Err(e) = client
            .resolve_batch(photos, db_pool, |current, total| {
                status.phases.geo_resolution.set_processed(current as u64);
                if current % 10 == 0 || current == total {
                    info!("Geo resolved {}/{}", current, total);
                }
            })
            .await
        {
            error!("Geo resolution failed: {}", e);
            status.phases.geo_resolution.add_errors(1);
            return;
        }

        info!("Phase 3 completed: {} photos geo resolved", total);
    }

    pub async fn run_startup_rescan(&self) -> Result<(), anyhow::Error> {
        self.run_cycle().await
    }

    /// The one full indexing cycle (the seven status phases). Startup,
    /// nightly and manual triggers all reach it; they differ only in how the
    /// run is started and reported.
    ///
    /// The lock and the status transitions live here too, so a phase-1 failure
    /// has exactly one meaning: mark the index incomplete, stop indexing and
    /// return the error. Later phases run only after phase 1 succeeded.
    async fn run_cycle(&self) -> Result<(), anyhow::Error> {
        let permit = match self.rescan_lock.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                warn!("Skipping rescan - another rescan is already in progress");
                return Ok(());
            }
        };

        self.begin_run().await;
        self.finish_cycle(permit).await
    }

    /// Starts the same full cycle on a dedicated indexing thread so an HTTP
    /// request never carries the scan. The status is switched to indexing
    /// before this returns — the `POST /api/indexing/scan` contract depends
    /// on it. When a run is already in progress, the caller attaches to it.
    pub async fn trigger_scan(&self) -> std::io::Result<ScanTrigger> {
        let Some(permit) = self.acquire_manual_scan_permit().await else {
            return Ok(ScanTrigger::AlreadyRunning);
        };

        self.begin_run().await;

        let scheduler = self.clone();
        let spawned = thread::Builder::new()
            .name("indexing-manual".into())
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("Failed to create indexing runtime");
                rt.block_on(async move {
                    if let Err(e) = scheduler.finish_cycle(permit).await {
                        error!("Manual rescan failed: {}", e);
                    }
                });
            });

        if let Err(e) = spawned {
            // No thread owns the permit or will clear the status; restore the
            // idle state and report the failure to the caller.
            self.status.stop_indexing().await;
            return Err(e);
        }

        Ok(ScanTrigger::Started)
    }

    /// Acquires the scan permit for a manual trigger. A mere `try_lock` miss
    /// is ambiguous: the holder may be starting (status not flipped yet) or
    /// finishing (status already stopped, permit not released yet). Waiting
    /// out that transition keeps the response honest — either it attaches to
    /// a run with `is_indexing=true`, or it starts a fresh run.
    async fn acquire_manual_scan_permit(&self) -> Option<OwnedSemaphorePermit> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            match self.rescan_lock.clone().try_acquire_owned() {
                Ok(permit) => return Some(permit),
                Err(_) if self.status.is_indexing.load(Ordering::SeqCst) => return None,
                Err(_) => {
                    if tokio::time::Instant::now() >= deadline {
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }
    }

    async fn begin_run(&self) {
        info!("Starting full photo rescan and cleanup");
        self.status.start_indexing().await;
        self.status.mark_incomplete();
    }

    /// Runs the phases after `begin_run` and owns the scan permit until the
    /// run has fully stopped.
    async fn finish_cycle(&self, permit: OwnedSemaphorePermit) -> Result<(), anyhow::Error> {
        let result = self.run_phases().await;

        match &result {
            Ok(()) if self.status.scan_complete.load(Ordering::SeqCst) => {
                self.status.mark_complete()
            }
            _ => self.status.mark_incomplete(),
        }
        // Always mark indexing as stopped, even on error.
        self.status.stop_indexing().await;
        drop(permit);
        result
    }

    async fn run_phases(&self) -> Result<(), anyhow::Error> {
        let processor = PhotoProcessor::new(self.photo_paths.clone(), self.semantic_search.clone());

        // Phase 1: Fast metadata-only scan (skip semantic vectors)
        info!("Phase 1: Fast metadata scan (skipping semantic vectors)");
        let processed_photos = processor
            .full_rescan_and_cleanup(&self.db_pool, &self.cache_manager, &self.status)
            .await
            .map_err(|e| anyhow::anyhow!("Phase 1 (metadata scan) failed: {}", e))?;

        // Convert ProcessedPhoto to Photo
        let photos: Vec<crate::db::Photo> =
            processed_photos.into_iter().map(|p| p.into()).collect();

        // Batch write photos to database
        let (indexed_count, error_count) =
            Self::batch_write_photos(&self.db_pool, photos, &self.status).await;

        info!(
            "Phase 1 completed: {} photos indexed, {} errors",
            indexed_count, error_count
        );

        // Phase 2: Batch compute semantic vectors
        info!("Phase 2: Computing semantic vectors in batches");
        self.status.set_phase("semantic_vectors").await;
        match processor
            .batch_compute_semantic_vectors(&self.db_pool, &self.status)
            .await
        {
            Ok((success, errors)) => {
                info!(
                    "Phase 2 completed: {} semantic vectors computed, {} errors",
                    success, errors
                )
            }
            Err(e) => error!("Phase 2 failed: {:#}", e),
        }

        // Phase 2b: Face detection/recognition. Disabled by the operator or
        // when the feature is off; the phase then leaves no counter work.
        if self.face_recognition.enabled() {
            info!("Phase 2b: Detecting and recognising faces");
            self.status.set_phase("faces").await;
            if let Err(e) = self.face_recognition.index_pending(&self.status).await {
                error!("Phase 2b (face recognition) failed: {:#}", e);
            }
        }

        // Phase 3: Geo resolution
        info!("Phase 3: Resolving photo geo locations");
        self.status.set_phase("geo_resolution").await;
        Self::run_geo_resolution_phase(&self.db_pool, &self.nominatim_url, &self.status).await;

        // Phase 4: Generate collages
        info!("Phase 4: Generating collages");
        self.status.set_phase("collages").await;
        match collage_generator::generate_collages(&self.db_pool, &self.data_path, &self.locale)
            .await
        {
            Ok(count) => info!("Phase 4 completed: {} collages generated", count),
            Err(e) => error!("Phase 4 (collage generation) failed: {}", e),
        }

        // Phase 5: Housekeeping Identification
        info!("Phase 5: Identifying housekeeping candidates");
        self.status.set_phase("housekeeping").await;
        match housekeeping_manager::run_housekeeping_scan(&self.db_pool, &self.semantic_search)
            .await
        {
            Ok(count) => info!(
                "Phase 5 completed: {} housekeeping candidates identified",
                count
            ),
            Err(e) => error!("Phase 5 (housekeeping identification) failed: {}", e),
        }

        info!(
            "Rescan completed (seven-phase): {} photos indexed, {} errors",
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

    use crate::cache_manager::CacheManager;
    use crate::db::{create_test_db_pool, DbPool, Photo};
    use crate::semantic_search::NoopSemanticSearch;

    fn get_cached_scheduler_model(pool: crate::db::DbPool) -> SemanticSearchEngine {
        let (loaded, device) = crate::semantic_search::cached_test_model();
        SemanticSearchEngine::new_with_model(loaded.model, loaded.tokenizer, device, pool, "./data")
    }

    struct TestEnvironment {
        temp_dir: TempDir,
        db_pool: DbPool,
        cache_manager: CacheManager,
        scheduler: PhotoScheduler,
    }

    impl TestEnvironment {
        async fn new_without_semantics() -> Self {
            Self::new_without_semantics_pool(false).await
        }

        /// The manual trigger runs on its own runtime, which an in-memory
        /// SQLite pool does not survive (its single connection is dropped with
        /// the runtime that created it). Production uses a file-backed DB, so
        /// the cross-runtime test must too.
        async fn new_without_semantics_file_backed() -> Self {
            Self::new_without_semantics_pool(true).await
        }

        async fn new_without_semantics_pool(file_backed: bool) -> Self {
            let temp_dir = TempDir::new().unwrap();
            let db_pool = if file_backed {
                let db_path = temp_dir.path().join("database").join("turbo-pix.db");
                crate::db_pool::create_db_pool(&db_path.to_string_lossy())
                    .await
                    .unwrap()
            } else {
                create_test_db_pool().await.unwrap()
            };
            let cache_manager = CacheManager::new(temp_dir.path().join("cache").to_path_buf());
            let semantic_search: Arc<dyn crate::semantic_search::SemanticSearch> =
                Arc::new(NoopSemanticSearch);

            let photo_paths = vec![temp_dir.path().to_path_buf()];
            let data_path = temp_dir.path().to_path_buf();

            let scheduler = PhotoScheduler::new(
                photo_paths,
                db_pool.clone(),
                cache_manager.clone(),
                Pipelines {
                    semantic_search,
                    face_recognition: Arc::new(crate::face_recognition::NoopFaceRecognition),
                },
                data_path,
                "en".to_string(),
                "https://nominatim.openstreetmap.org".to_string(),
            );

            Self {
                temp_dir,
                db_pool,
                cache_manager,
                scheduler,
            }
        }

        async fn new_with_semantics() -> Self {
            let temp_dir = TempDir::new().unwrap();
            let db_pool = create_test_db_pool().await.unwrap();
            let cache_manager = CacheManager::new(temp_dir.path().join("cache").to_path_buf());
            let semantic_search: Arc<dyn crate::semantic_search::SemanticSearch> =
                Arc::new(get_cached_scheduler_model(db_pool.clone()));

            let photo_paths = vec![temp_dir.path().to_path_buf()];
            let data_path = temp_dir.path().to_path_buf();

            let scheduler = PhotoScheduler::new(
                photo_paths,
                db_pool.clone(),
                cache_manager.clone(),
                Pipelines {
                    semantic_search,
                    face_recognition: Arc::new(crate::face_recognition::NoopFaceRecognition),
                },
                data_path,
                "en".to_string(),
                "https://nominatim.openstreetmap.org".to_string(),
            );

            Self {
                temp_dir,
                db_pool,
                cache_manager,
                scheduler,
            }
        }

        async fn new() -> Self {
            Self::new_without_semantics().await
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
                taken_latitude: None,
                taken_longitude: None,
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

            photo.create_or_update(&self.db_pool).await?;
            Ok(())
        }

        async fn get_all_photo_paths(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
            crate::db::get_all_photo_paths(&self.db_pool).await
        }
    }

    #[tokio::test]
    async fn test_photo_scheduler_new() {
        let env = TestEnvironment::new().await;
        assert!(!env.scheduler.photo_paths.is_empty());
    }

    #[test]
    fn test_run_startup_rescan_executes_in_dedicated_runtime() {
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let env = TestEnvironment::new_without_semantics().await;
            env.scheduler.run_startup_rescan().await
        });

        assert!(result.is_ok());
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
        let env = TestEnvironment::new_without_semantics().await;

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
        let env = TestEnvironment::new_without_semantics().await;

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
        let db_pool = create_test_db_pool().await.unwrap();

        // Test vacuum operation (should not fail, even on empty DB)
        let vacuum_result = crate::db::vacuum_database(&db_pool).await;
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
        let env = TestEnvironment::new_without_semantics().await;

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
        let db_pool = create_test_db_pool().await.unwrap();
        let cache_manager = CacheManager::new(temp_dir.path().join("cache").to_path_buf());
        let semantic_search: Arc<dyn crate::semantic_search::SemanticSearch> =
            Arc::new(NoopSemanticSearch);
        let data_path = temp_dir.path().to_path_buf();

        // Create scheduler with non-existent path
        let invalid_paths = vec![PathBuf::from("/nonexistent/path")];
        let scheduler = PhotoScheduler::new(
            invalid_paths,
            db_pool,
            cache_manager,
            Pipelines {
                semantic_search,
                face_recognition: Arc::new(crate::face_recognition::NoopFaceRecognition),
            },
            data_path,
            "en".to_string(),
            "https://nominatim.openstreetmap.org".to_string(),
        );

        // Should handle errors gracefully
        let result = scheduler.run_startup_rescan().await;
        assert!(result.is_ok()); // Should not panic, just log errors
        assert!(
            !scheduler.status.is_complete.load(Ordering::SeqCst),
            "a missing photo root makes the scan partial, never complete"
        );
    }

    #[tokio::test]
    async fn test_trigger_scan_is_single_flight_and_reports_started() {
        let env = TestEnvironment::new_without_semantics_file_backed().await;
        env.create_test_image("manual-scan.jpg", b"manual scan content");

        // Hold the scan permit and a running status to prove a second trigger
        // attaches instead of spawning a second run.
        let held_permit = env
            .scheduler
            .rescan_lock
            .clone()
            .try_acquire_owned()
            .expect("test must own the only permit");
        env.scheduler.status.start_indexing().await;
        assert_eq!(
            env.scheduler.trigger_scan().await.unwrap(),
            ScanTrigger::AlreadyRunning
        );
        assert!(
            env.scheduler.status.is_indexing.load(Ordering::SeqCst),
            "an attached response must report the run it attached to"
        );

        // A runner that has stopped indexing but not yet released the permit
        // must not produce a phantom attach; the trigger waits for the
        // release and starts a fresh run.
        env.scheduler.status.stop_indexing().await;
        let release_delay = tokio::spawn(async move {
            sleep(TokioDuration::from_millis(20)).await;
            drop(held_permit);
        });
        assert_eq!(
            env.scheduler.trigger_scan().await.unwrap(),
            ScanTrigger::Started
        );
        release_delay.await.unwrap();
        assert!(
            env.scheduler.status.is_indexing.load(Ordering::SeqCst),
            "the 202 response depends on is_indexing being true immediately"
        );

        for _ in 0..400 {
            if !env.scheduler.status.is_indexing.load(Ordering::SeqCst) {
                break;
            }
            sleep(TokioDuration::from_millis(25)).await;
        }
        assert!(!env.scheduler.status.is_indexing.load(Ordering::SeqCst));
        assert!(env.scheduler.status.is_complete.load(Ordering::SeqCst));
        assert_eq!(env.get_all_photo_paths().await.unwrap().len(), 1);

        assert_eq!(
            env.scheduler.trigger_scan().await.unwrap(),
            ScanTrigger::Started,
            "the permit must be released after the manual run"
        );
        for _ in 0..400 {
            if !env.scheduler.status.is_indexing.load(Ordering::SeqCst) {
                break;
            }
            sleep(TokioDuration::from_millis(25)).await;
        }
    }

    #[tokio::test]
    async fn test_concurrent_rescan_operations() {
        let env = TestEnvironment::new_without_semantics().await;

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
        let env = TestEnvironment::new_with_semantics().await;

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
        let processor = PhotoProcessor::new(
            env.scheduler.photo_paths.clone(),
            env.scheduler.semantic_search.clone(),
        );
        let result = processor
            .batch_compute_semantic_vectors(&env.db_pool, &env.scheduler.status)
            .await;

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

        let snapshot = env.scheduler.status.phases.semantic_vectors.snapshot();
        assert_eq!(snapshot.total, 3);
        assert_eq!(snapshot.processed + snapshot.errors, 3);
        assert!(snapshot.current_item.is_none());
    }

    #[tokio::test]
    async fn test_batch_write_consistency() {
        let env = TestEnvironment::new_without_semantics().await;

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
