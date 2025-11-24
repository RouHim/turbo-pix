use log::info;
use std::path::PathBuf;
use warp::{reject, Filter, Rejection, Reply};

use crate::collage_generator::{self, Collage};
use crate::db::DbPool;
use crate::warp_helpers::{with_db, DatabaseError};

/// List all pending collages
pub async fn list_pending_collages(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Collage::list_pending(&db_pool) {
        Ok(collages) => Ok(warp::reply::json(&collages)),
        Err(e) => {
            log::error!("Failed to list pending collages: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

/// Accept a collage (move to photos directory and trigger indexing)
pub async fn accept_collage(
    id: i64,
    db_pool: DbPool,
    data_path: PathBuf,
) -> Result<impl Reply, Rejection> {
    info!("Accepting collage {}", id);

    // Move collage to photos directory
    let accepted_path = match collage_generator::accept_collage(&db_pool, id, &data_path).await {
        Ok(path) => path,
        Err(e) => {
            log::error!("Failed to accept collage: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to accept collage: {}", e),
            }));
        }
    };

    info!("Collage accepted and moved to {:?}", accepted_path);

    // The collage will be indexed in the next nightly rescan
    // Immediate indexing is not critical for collages
    Ok(warp::reply::json(&serde_json::json!({
        "success": true,
        "message": "Collage accepted. It will appear in 'All Photos' after the next scan.",
        "path": accepted_path.to_string_lossy()
    })))
}

/// Reject a collage (delete files and database record)
pub async fn reject_collage(id: i64, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    info!("Rejecting collage {}", id);

    match collage_generator::reject_collage(&db_pool, id).await {
        Ok(_) => Ok(warp::reply::json(&serde_json::json!({
            "success": true,
            "message": "Collage rejected and deleted"
        }))),
        Err(e) => {
            log::error!("Failed to reject collage: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to reject collage: {}", e),
            }))
        }
    }
}

/// Manually trigger collage generation (for testing)
pub async fn generate_collages_manual(
    db_pool: DbPool,
    data_path: PathBuf,
) -> Result<impl Reply, Rejection> {
    info!("Manual collage generation triggered");

    match collage_generator::generate_collages(&db_pool, &data_path).await {
        Ok(count) => {
            info!(
                "Manual collage generation completed: {} collages created",
                count
            );
            Ok(warp::reply::json(&serde_json::json!({
                "success": true,
                "count": count,
                "message": format!("{} collage(s) generated", count)
            })))
        }
        Err(e) => {
            log::error!("Failed to generate collages: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to generate collages: {}", e),
            }))
        }
    }
}

/// Build collage routes
pub fn build_collage_routes(
    db_pool: DbPool,
    data_path: PathBuf,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    let list_pending = warp::path!("api" / "collages" / "pending")
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(list_pending_collages);

    let data_path_generate = data_path.clone();
    let generate = warp::path!("api" / "collages" / "generate")
        .and(warp::post())
        .and(with_db(db_pool.clone()))
        .map(move |db_pool| (db_pool, data_path_generate.clone()))
        .untuple_one()
        .and_then(generate_collages_manual);

    let data_path_accept = data_path;
    let accept = warp::path!("api" / "collages" / i64 / "accept")
        .and(warp::post())
        .and(with_db(db_pool.clone()))
        .map(move |id, db_pool| (id, db_pool, data_path_accept.clone()))
        .untuple_one()
        .and_then(accept_collage);

    let reject = warp::path!("api" / "collages" / i64 / "reject")
        .and(warp::delete())
        .and(with_db(db_pool))
        .and_then(reject_collage);

    list_pending.or(generate).or(accept).or(reject)
}
