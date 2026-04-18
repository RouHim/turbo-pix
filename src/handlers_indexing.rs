use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use warp::Filter;

use crate::db::DbPool;
use crate::scheduler::{IndexingPhases, IndexingStatus};
use crate::warp_helpers::with_db;

const CANONICAL_PHASES: [(&str, &str); 6] = [
    ("discovering", "indeterminate"),
    ("metadata", "determinate"),
    ("semantic_vectors", "determinate"),
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

pub async fn get_indexing_status(
    status: IndexingStatus,
    db_pool: DbPool,
) -> Result<impl warp::Reply, Infallible> {
    let is_indexing = status.is_indexing.load(Ordering::SeqCst);
    let is_complete = status.is_complete.load(Ordering::SeqCst);
    let current_phase = status.current_phase.lock().await.clone();
    let started_at = status.started_at.lock().await.map(|dt| dt.to_rfc3339());
    let photos_indexed = get_total_photo_count(&db_pool).await;

    let active_phase_id = if is_indexing {
        current_phase.clone()
    } else {
        String::new()
    };

    let phases = build_phases(is_indexing, is_complete, &current_phase, &status.phases);

    let response = IndexingStatusResponse {
        is_indexing,
        is_complete,
        started_at,
        active_phase_id,
        phases,
        photos_indexed,
    };

    let reply = warp::reply::json(&response);
    let reply = warp::reply::with_header(
        reply,
        "cache-control",
        "no-store, no-cache, must-revalidate",
    );
    let reply = warp::reply::with_header(reply, "pragma", "no-cache");

    Ok(reply)
}

fn with_indexing_status(
    status: IndexingStatus,
) -> impl Filter<Extract = (IndexingStatus,), Error = Infallible> + Clone {
    warp::any().map(move || status.clone())
}

pub fn build_indexing_routes(
    status: IndexingStatus,
    db_pool: DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("api" / "indexing" / "status")
        .and(warp::get())
        .and(with_indexing_status(status))
        .and(with_db(db_pool))
        .and_then(get_indexing_status)
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

    #[test]
    fn test_build_phases_geo_resolution_active() {
        let data = make_phases((500, 500), (500, 500), (500, 500));
        let phases = build_phases(true, false, "geo_resolution", &data);
        assert_eq!(phases.len(), 6);
        assert!(
            phases[0..3].iter().all(|p| p.state == "done"),
            "discovering, metadata, semantic_vectors should be done"
        );
        assert_eq!(phases[3].state, "active");
        assert_eq!(phases[3].kind, "determinate");
        assert_eq!(phases[3].id, "geo_resolution");
        assert!(
            phases[4..6].iter().all(|p| p.state == "pending"),
            "collages and housekeeping should be pending"
        );
    }

    #[test]
    fn test_canonical_phases_includes_geo_resolution() {
        assert_eq!(CANONICAL_PHASES.len(), 6);
        assert!(
            CANONICAL_PHASES
                .iter()
                .any(|(id, _)| *id == "geo_resolution"),
            "geo_resolution must be in CANONICAL_PHASES"
        );
        assert_eq!(
            CANONICAL_PHASES[3],
            ("geo_resolution", "determinate"),
            "geo_resolution must be at index 3 (after semantic_vectors, before collages)"
        );
    }

    #[test]
    fn test_build_phases_idle() {
        let data = IndexingPhases::new();
        let phases = build_phases(false, false, "idle", &data);
        assert_eq!(phases.len(), 6);
        assert!(phases.iter().all(|p| p.state == "pending"));
        assert_eq!(phases[0].id, "discovering");
        assert_eq!(phases[1].id, "metadata");
        assert_eq!(phases[2].id, "semantic_vectors");
        assert_eq!(phases[3].id, "geo_resolution");
        assert_eq!(phases[4].id, "collages");
        assert_eq!(phases[5].id, "housekeeping");
    }

    #[test]
    fn test_build_phases_complete() {
        let data = make_phases((100, 100), (100, 100), (100, 100));
        let phases = build_phases(false, true, "idle", &data);
        assert_eq!(phases.len(), 6);
        assert!(phases.iter().all(|p| p.state == "done"));
        assert_eq!(phases[1].processed, 100);
        assert_eq!(phases[1].total, Some(100));
        assert_eq!(phases[2].processed, 100);
        assert_eq!(phases[2].total, Some(100));
        assert_eq!(phases[3].processed, 100);
        assert_eq!(phases[3].total, Some(100));
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
    }

    #[test]
    fn test_build_phases_collages_active() {
        let data = make_phases((500, 500), (500, 500), (100, 100));
        let phases = build_phases(true, false, "collages", &data);
        assert_eq!(phases[0].state, "done");
        assert_eq!(phases[1].state, "done");
        assert_eq!(phases[2].state, "done");
        assert_eq!(phases[3].state, "done");
        assert_eq!(phases[4].state, "active");
        assert_eq!(phases[4].kind, "indeterminate");
        assert_eq!(phases[4].total, None);
        assert_eq!(phases[5].state, "pending");
    }

    #[test]
    fn test_build_phases_housekeeping_active() {
        let data = make_phases((500, 500), (500, 500), (100, 100));
        let phases = build_phases(true, false, "housekeeping", &data);
        assert!(phases[0..5].iter().all(|p| p.state == "done"));
        assert_eq!(phases[5].state, "active");
        assert_eq!(phases[5].kind, "indeterminate");
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
        assert_eq!(phases[4].kind, "indeterminate");
        assert_eq!(phases[5].kind, "indeterminate");
    }
}
