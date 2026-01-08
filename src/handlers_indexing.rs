use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use warp::Filter;

use crate::db::DbPool;
use crate::scheduler::IndexingStatus;
use crate::warp_helpers::with_db;

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexingStatusResponse {
    pub is_indexing: bool,
    pub is_complete: bool,
    pub photos_indexed: u64,
    pub phase: String,
    pub photos_total: u64,
    pub photos_processed: u64,
    pub photos_semantic_indexed: u64,
    pub started_at: Option<String>,
    pub progress_percent: f64,
}

/// Helper function to get the total count of photos in the database
async fn get_total_photo_count(db_pool: &DbPool) -> u64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM photos")
        .fetch_one(db_pool)
        .await
        .unwrap_or(0) as u64
}

pub async fn get_indexing_status(
    status: IndexingStatus,
    db_pool: DbPool,
) -> Result<impl warp::Reply, Infallible> {
    let is_indexing = status.is_indexing.load(Ordering::SeqCst);
    let is_complete = status.is_complete.load(Ordering::SeqCst);
    let phase = status.current_phase.lock().await.clone();
    let photos_total = status.photos_total.load(Ordering::SeqCst);
    let photos_processed = status.photos_processed.load(Ordering::SeqCst);
    let photos_semantic_indexed = status.photos_semantic_indexed.load(Ordering::SeqCst);
    let started_at = status.started_at.lock().await.map(|dt| dt.to_rfc3339());

    // Get total photos in database
    let photos_indexed = get_total_photo_count(&db_pool).await;

    // Calculate progress percentage
    let progress_percent = if photos_total > 0 {
        match phase.as_str() {
            "metadata" => (photos_processed as f64 / photos_total as f64) * 100.0,
            "semantic_vectors" => {
                // Phase 1 is 50%, Phase 2 is the other 50%
                let phase1_progress = 50.0;
                let phase2_progress = if photos_total > 0 {
                    (photos_semantic_indexed as f64 / photos_total as f64) * 50.0
                } else {
                    0.0
                };
                phase1_progress + phase2_progress
            }
            _ => 0.0,
        }
    } else {
        0.0
    };

    let response = IndexingStatusResponse {
        is_indexing,
        is_complete,
        photos_indexed,
        phase,
        photos_total,
        photos_processed,
        photos_semantic_indexed,
        started_at,
        progress_percent,
    };

    Ok(warp::reply::json(&response))
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
