use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use warp::reply::Reply;
use warp::Filter;

use crate::db::DbPool;
use crate::scheduler::{IndexingPhases, IndexingStatus, PhotoScheduler};
use crate::warp_helpers::with_db;

/// Spawning the dedicated indexing thread failed (e.g. resource exhaustion).
/// Handled by the generic 500 arm.
#[derive(Debug)]
struct ScanStartError;

impl warp::reject::Reject for ScanStartError {}

const CANONICAL_PHASES: [(&str, &str); 7] = [
    ("discovering", "indeterminate"),
    ("metadata", "determinate"),
    ("semantic_vectors", "determinate"),
    ("faces", "determinate"),
    ("geo_resolution", "determinate"),
    ("collages", "indeterminate"),
    ("housekeeping", "indeterminate"),
];

#[derive(Debug, Serialize, Deserialize)]
pub struct PhaseProgress {
    pub id: String,
    /// pending | active | done | error
    pub state: String,
    /// determinate | indeterminate
    pub kind: String,
    pub processed: u64,
    /// null when kind is indeterminate or phase hasn't started
    pub total: Option<u64>,
    pub errors: u64,
    /// Raw path/basename of the item currently being processed (not user-facing text)
    pub current_item: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexingStatusResponse {
    pub is_indexing: bool,
    pub is_complete: bool,
    pub started_at: Option<String>,
    pub active_phase_id: String,
    pub phases: Vec<PhaseProgress>,
    /// Total photos in the database (kept for backward compatibility during frontend migration)
    pub photos_indexed: u64,
}

async fn get_total_photo_count(db_pool: &DbPool) -> u64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM photos")
        .fetch_one(db_pool)
        .await
        .unwrap_or(0) as u64
}

/// Build the phases array from current indexing state.
///
/// Phase state is derived from position relative to the active phase:
/// - Before active -> done
/// - At active -> active
/// - After active -> pending
///
/// When not indexing: all phases are either "done" (if complete) or "pending" (if idle).
fn build_phases(
    is_indexing: bool,
    is_complete: bool,
    current_phase: &str,
    phases_data: &IndexingPhases,
) -> Vec<PhaseProgress> {
    let active_idx = CANONICAL_PHASES
        .iter()
        .position(|(id, _)| *id == current_phase);

    fn state_for(
        i: usize,
        active_idx: Option<usize>,
        is_indexing: bool,
        is_complete: bool,
    ) -> &'static str {
        if is_indexing {
            match active_idx {
                Some(active) if i < active => "done",
                Some(active) if i == active => "active",
                _ => "pending",
            }
        } else if is_complete {
            "done"
        } else {
            "pending"
        }
    }

    CANONICAL_PHASES
        .iter()
        .enumerate()
        .map(|(i, (id, kind))| {
            let state = state_for(i, active_idx, is_indexing, is_complete);

            let snap = phases_data
                .get(id)
                .map(|c| c.snapshot())
                .unwrap_or_default();

            let (processed, total) = match (*kind, state) {
                ("determinate", "active" | "done") if snap.total > 0 => {
                    (snap.processed, Some(snap.total))
                }
                _ => (snap.processed, None),
            };

            PhaseProgress {
                id: id.to_string(),
                state: state.to_string(),
                kind: kind.to_string(),
                processed,
                total,
                errors: snap.errors,
                current_item: snap.current_item,
            }
        })
        .collect()
}

async fn build_status_response(
    status: &IndexingStatus,
    db_pool: &DbPool,
) -> IndexingStatusResponse {
    let is_indexing = status.is_indexing.load(Ordering::SeqCst);
    let is_complete = status.is_complete.load(Ordering::SeqCst);
    let current_phase = status.current_phase.lock().await.clone();
    let started_at = status.started_at.lock().await.map(|dt| dt.to_rfc3339());
    let photos_indexed = get_total_photo_count(db_pool).await;

    let active_phase_id = if is_indexing {
        current_phase.clone()
    } else {
        String::new()
    };

    let phases = build_phases(is_indexing, is_complete, &current_phase, &status.phases);

    IndexingStatusResponse {
        is_indexing,
        is_complete,
        started_at,
        active_phase_id,
        phases,
        photos_indexed,
    }
}

fn status_reply(
    response: &IndexingStatusResponse,
    status: warp::http::StatusCode,
) -> warp::reply::Response {
    let reply = warp::reply::json(response);
    let reply = warp::reply::with_status(reply, status);
    let reply = warp::reply::with_header(
        reply,
        "cache-control",
        "no-store, no-cache, must-revalidate",
    );
    warp::reply::with_header(reply, "pragma", "no-cache").into_response()
}

pub async fn get_indexing_status(
    status: IndexingStatus,
    db_pool: DbPool,
) -> Result<warp::reply::Response, warp::Rejection> {
    let response = build_status_response(&status, &db_pool).await;
    Ok(status_reply(&response, warp::http::StatusCode::OK))
}

/// Starts the full indexing cycle in the background. A second request while a
/// cycle is running attaches to it and returns the same accepted status.
pub async fn trigger_scan(
    scheduler: PhotoScheduler,
    db_pool: DbPool,
) -> Result<warp::reply::Response, warp::Rejection> {
    match scheduler.trigger_scan().await {
        Ok(_) => {
            let mut response = build_status_response(&scheduler.status, &db_pool).await;
            // A tiny cycle can finish between the spawn and this snapshot; the
            // endpoint's contract is the accepted scan, and the client's
            // immediate poll reconciles the final state.
            response.is_indexing = true;
            Ok(status_reply(&response, warp::http::StatusCode::ACCEPTED))
        }
        Err(e) => {
            log::error!("Failed to spawn manual scan: {}", e);
            Err(warp::reject::custom(ScanStartError))
        }
    }
}

fn with_indexing_status(
    status: IndexingStatus,
) -> impl Filter<Extract = (IndexingStatus,), Error = Infallible> + Clone {
    warp::any().map(move || status.clone())
}

fn with_scheduler(
    scheduler: PhotoScheduler,
) -> impl Filter<Extract = (PhotoScheduler,), Error = Infallible> + Clone {
    warp::any().map(move || scheduler.clone())
}

pub fn build_indexing_routes(
    scheduler: PhotoScheduler,
    db_pool: DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let status = scheduler.status.clone();
    warp::path!("api" / "indexing" / "status")
        .and(warp::get())
        .and(with_indexing_status(status))
        .and(with_db(db_pool.clone()))
        .and_then(get_indexing_status)
        .or(warp::path!("api" / "indexing" / "scan")
            .and(warp::post())
            .and(with_scheduler(scheduler))
            .and(with_db(db_pool))
            .and_then(trigger_scan))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_phases(metadata: (u64, u64), semantic: (u64, u64), geo: (u64, u64)) -> IndexingPhases {
        let p = IndexingPhases::new();
        p.metadata.set_total(metadata.1);
        p.metadata.set_processed(metadata.0);
        p.semantic_vectors.set_total(semantic.1);
        p.semantic_vectors.set_processed(semantic.0);
        p.geo_resolution.set_total(geo.1);
        p.geo_resolution.set_processed(geo.0);
        p
    }

    async fn test_routes() -> (
        tempfile::TempDir,
        PhotoScheduler,
        impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone,
    ) {
        use crate::cache_manager::CacheManager;
        use crate::db_pool::create_db_pool;
        use crate::face_recognition::NoopFaceRecognition;
        use crate::scheduler::Pipelines;
        use crate::semantic_search::NoopSemanticSearch;
        use crate::warp_helpers::handle_rejection;
        use std::sync::Arc;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("database").join("turbo-pix.db");
        let db_pool = create_db_pool(&db_path.to_string_lossy()).await.unwrap();
        let scheduler = PhotoScheduler::new(
            vec![temp_dir.path().to_path_buf()],
            db_pool.clone(),
            CacheManager::new(temp_dir.path().join("cache")),
            Pipelines {
                semantic_search: Arc::new(NoopSemanticSearch),
                face_recognition: Arc::new(NoopFaceRecognition),
            },
            temp_dir.path().to_path_buf(),
            "en".to_string(),
            "https://nominatim.openstreetmap.org".to_string(),
        );

        let routes = build_indexing_routes(scheduler.clone(), db_pool).recover(handle_rejection);
        // The fixture must outlive the scan started by POST.
        (temp_dir, scheduler, routes)
    }

    #[tokio::test]
    async fn test_post_scan_returns_accepted_status() {
        let (_temp_dir, scheduler, routes) = test_routes().await;

        let response = warp::test::request()
            .method("POST")
            .path("/api/indexing/scan")
            .reply(&routes)
            .await;
        assert_eq!(response.status(), warp::http::StatusCode::ACCEPTED);
        let body: IndexingStatusResponse = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body.phases.len(), 7);

        let response = warp::test::request()
            .method("GET")
            .path("/api/indexing/status")
            .reply(&routes)
            .await;
        assert_eq!(response.status(), warp::http::StatusCode::OK);

        for _ in 0..400 {
            if !scheduler.status.is_indexing.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(!scheduler.status.is_indexing.load(Ordering::SeqCst));
    }

    #[test]
    fn test_build_phases_geo_resolution_active() {
        let data = make_phases((500, 500), (500, 500), (500, 500));
        let phases = build_phases(true, false, "geo_resolution", &data);
        assert_eq!(phases.len(), 7);
        assert!(
            phases[0..4].iter().all(|p| p.state == "done"),
            "discovering, metadata, semantic_vectors and faces should be done"
        );
        assert_eq!(phases[4].state, "active");
        assert_eq!(phases[4].kind, "determinate");
        assert_eq!(phases[4].id, "geo_resolution");
        assert!(
            phases[5..7].iter().all(|p| p.state == "pending"),
            "collages and housekeeping should be pending"
        );
    }

    #[test]
    fn test_canonical_phases_includes_geo_resolution() {
        assert_eq!(CANONICAL_PHASES.len(), 7);
        assert!(
            CANONICAL_PHASES
                .iter()
                .any(|(id, _)| *id == "geo_resolution"),
            "geo_resolution must be in CANONICAL_PHASES"
        );
        assert_eq!(
            CANONICAL_PHASES[4],
            ("geo_resolution", "determinate"),
            "geo_resolution must be at index 4 (after faces, before collages)"
        );
        assert_eq!(
            CANONICAL_PHASES[3],
            ("faces", "determinate"),
            "faces must run after semantic_vectors"
        );
    }

    #[test]
    fn test_build_phases_idle() {
        let data = IndexingPhases::new();
        let phases = build_phases(false, false, "idle", &data);
        assert_eq!(phases.len(), 7);
        assert!(phases.iter().all(|p| p.state == "pending"));
        assert_eq!(phases[0].id, "discovering");
        assert_eq!(phases[1].id, "metadata");
        assert_eq!(phases[2].id, "semantic_vectors");
        assert_eq!(phases[3].id, "faces");
        assert_eq!(phases[4].id, "geo_resolution");
        assert_eq!(phases[5].id, "collages");
        assert_eq!(phases[6].id, "housekeeping");
    }

    #[test]
    fn test_build_phases_complete() {
        let data = make_phases((100, 100), (100, 100), (100, 100));
        let phases = build_phases(false, true, "idle", &data);
        assert_eq!(phases.len(), 7);
        assert!(phases.iter().all(|p| p.state == "done"));
        assert_eq!(phases[1].processed, 100);
        assert_eq!(phases[1].total, Some(100));
        assert_eq!(phases[2].processed, 100);
        assert_eq!(phases[2].total, Some(100));
        assert_eq!(phases[4].processed, 100);
        assert_eq!(phases[4].total, Some(100));
    }

    #[test]
    fn test_build_phases_metadata_active() {
        let data = make_phases((123, 500), (0, 0), (0, 0));
        let phases = build_phases(true, false, "metadata", &data);
        assert_eq!(phases[0].state, "done");
        assert_eq!(phases[1].state, "active");
        assert_eq!(phases[1].processed, 123);
        assert_eq!(phases[1].total, Some(500));
        assert_eq!(phases[1].kind, "determinate");
        assert_eq!(phases[2].state, "pending");
        assert_eq!(phases[3].state, "pending");
        assert_eq!(phases[4].state, "pending");
        assert_eq!(phases[5].state, "pending");
        assert_eq!(phases[6].state, "pending");
    }

    #[test]
    fn test_build_phases_semantic_active() {
        let data = make_phases((500, 500), (250, 500), (0, 0));
        let phases = build_phases(true, false, "semantic_vectors", &data);
        assert_eq!(phases[0].state, "done");
        assert_eq!(phases[1].state, "done");
        assert_eq!(phases[1].processed, 500);
        assert_eq!(phases[1].total, Some(500));
        assert_eq!(phases[2].state, "active");
        assert_eq!(phases[2].processed, 250);
        assert_eq!(phases[2].total, Some(500));
        assert_eq!(phases[2].kind, "determinate");
        assert_eq!(phases[3].state, "pending");
        assert_eq!(phases[4].state, "pending");
        assert_eq!(phases[5].state, "pending");
        assert_eq!(phases[6].state, "pending");
    }

    #[test]
    fn test_build_phases_collages_active() {
        let data = make_phases((500, 500), (500, 500), (100, 100));
        let phases = build_phases(true, false, "collages", &data);
        assert_eq!(phases[0].state, "done");
        assert_eq!(phases[1].state, "done");
        assert_eq!(phases[2].state, "done");
        assert_eq!(phases[3].state, "done");
        assert_eq!(phases[4].state, "done");
        assert_eq!(phases[5].state, "active");
        assert_eq!(phases[5].kind, "indeterminate");
        assert_eq!(phases[5].total, None);
        assert_eq!(phases[6].state, "pending");
    }

    #[test]
    fn test_build_phases_housekeeping_active() {
        let data = make_phases((500, 500), (500, 500), (100, 100));
        let phases = build_phases(true, false, "housekeeping", &data);
        assert!(phases[0..6].iter().all(|p| p.state == "done"));
        assert_eq!(phases[6].state, "active");
        assert_eq!(phases[6].kind, "indeterminate");
    }

    #[test]
    fn test_build_phases_discovering_active() {
        let data = IndexingPhases::new();
        let phases = build_phases(true, false, "discovering", &data);
        assert_eq!(phases[0].state, "active");
        assert_eq!(phases[0].kind, "indeterminate");
        assert_eq!(phases[0].total, None);
        assert!(phases[1..].iter().all(|p| p.state == "pending"));
    }

    #[test]
    fn test_build_phases_pending_determinate_has_no_total() {
        let data = IndexingPhases::new();
        let phases = build_phases(true, false, "discovering", &data);
        assert_eq!(phases[1].total, None);
        assert_eq!(phases[2].total, None);
        assert_eq!(phases[3].total, None);
    }

    #[test]
    fn test_canonical_phase_ids() {
        let data = IndexingPhases::new();
        let phases = build_phases(false, false, "idle", &data);
        let ids: Vec<&str> = phases.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "discovering",
                "metadata",
                "semantic_vectors",
                "faces",
                "geo_resolution",
                "collages",
                "housekeeping"
            ]
        );
    }

    #[test]
    fn test_phase_kinds() {
        let data = IndexingPhases::new();
        let phases = build_phases(false, false, "idle", &data);
        assert_eq!(phases[0].kind, "indeterminate");
        assert_eq!(phases[1].kind, "determinate");
        assert_eq!(phases[2].kind, "determinate");
        assert_eq!(phases[3].kind, "determinate");
        assert_eq!(phases[4].kind, "determinate");
        assert_eq!(phases[5].kind, "indeterminate");
        assert_eq!(phases[6].kind, "indeterminate");
    }
}
