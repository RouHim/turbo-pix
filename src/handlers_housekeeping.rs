use serde::Serialize;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::{DbPool, Photo};
use crate::warp_helpers::{with_db, DatabaseError, NotFoundError};

#[derive(Debug, Serialize)]
pub struct HousekeepingCandidate {
    pub photo: Photo,
    pub reason: String,
    pub score: f32,
}

#[derive(Debug, Serialize)]
pub struct HousekeepingResponse {
    pub candidates: Vec<HousekeepingCandidate>,
}

pub async fn list_housekeeping_candidates(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    // Query candidates with photo hashes and metadata
    let candidates_data: Vec<(String, String, f32)> = sqlx::query_as(
        "SELECT photo_hash, reason, score
         FROM housekeeping_candidates
         ORDER BY score DESC
         LIMIT 100",
    )
    .fetch_all(&db_pool)
    .await
    .map_err(|e| {
        reject::custom(DatabaseError {
            message: format!("Failed to fetch candidates: {}", e),
        })
    })?;

    // Fetch photos for each candidate
    let mut candidates = Vec::new();
    for (photo_hash, reason, score) in candidates_data {
        if let Ok(Some(photo)) =
            sqlx::query_as::<_, Photo>("SELECT * FROM photos WHERE hash_sha256 = ?")
                .bind(&photo_hash)
                .fetch_optional(&db_pool)
                .await
        {
            candidates.push(HousekeepingCandidate {
                photo,
                reason,
                score,
            });
        }
    }

    Ok(warp::reply::json(&HousekeepingResponse { candidates }))
}

pub async fn remove_housekeeping_candidate(
    hash: String,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let result = sqlx::query("DELETE FROM housekeeping_candidates WHERE photo_hash = ?")
        .bind(&hash)
        .execute(&db_pool)
        .await
        .map_err(|e| {
            reject::custom(DatabaseError {
                message: format!("Failed to delete candidate: {}", e),
            })
        })?;

    if result.rows_affected() == 0 {
        return Err(reject::custom(NotFoundError));
    }

    Ok(warp::reply::json(&serde_json::json!({ "success": true })))
}

pub fn build_housekeeping_routes(
    db_pool: DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let list_route = warp::path("api")
        .and(warp::path("housekeeping"))
        .and(warp::path("candidates"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(list_housekeeping_candidates);

    let remove_route = warp::path("api")
        .and(warp::path("housekeeping"))
        .and(warp::path("candidates"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and_then(remove_housekeeping_candidate);

    list_route.or(remove_route)
}
