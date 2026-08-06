use serde::Serialize;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::{DbPool, Photo};
use crate::handlers_photo::{validate_hashes, BatchFailure, BatchHashesRequest, BatchResult};
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

/// Batch-dismiss housekeeping candidates ("keep"): the photos themselves stay
/// in the library. Partial failure is a 200 with a per-item failure list.
pub async fn batch_remove_candidates(
    req: BatchHashesRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    validate_hashes(&req.hashes)?;

    let mut result = BatchResult {
        applied: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
    };

    for hash in &req.hashes {
        let outcome = sqlx::query("DELETE FROM housekeeping_candidates WHERE photo_hash = ?")
            .bind(hash)
            .execute(&db_pool)
            .await;
        match outcome {
            Ok(query_result) if query_result.rows_affected() > 0 => {
                result.applied.push(hash.clone());
            }
            Ok(_) => result.failed.push(BatchFailure {
                id: hash.clone(),
                error: "Candidate not found".to_string(),
            }),
            Err(e) => {
                log::error!("Failed to delete candidate {}: {}", hash, e);
                result.failed.push(BatchFailure {
                    id: hash.clone(),
                    error: format!("Database error: {}", e),
                });
            }
        }
    }

    Ok(warp::reply::json(&result))
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

    // Literal batch route must be registered BEFORE the parameterized
    // remove_route: `/api/housekeeping/candidates/batch-remove` would
    // otherwise be matched as `remove_housekeeping_candidate("batch-remove")`
    // and 404 with a misleading message.
    let batch_remove_route = warp::path("api")
        .and(warp::path("housekeeping"))
        .and(warp::path("candidates"))
        .and(warp::path("batch-remove"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::body::content_length_limit(1024 * 1024))
        .and(warp::body::json::<BatchHashesRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(batch_remove_candidates);

    let remove_route = warp::path("api")
        .and(warp::path("housekeeping"))
        .and(warp::path("candidates"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and_then(remove_housekeeping_candidate);

    list_route.or(batch_remove_route).or(remove_route)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_in_memory_pool;
    use crate::warp_helpers::handle_rejection;
    use std::convert::Infallible;

    fn build_test_routes(
        db_pool: DbPool,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        build_housekeeping_routes(db_pool).recover(handle_rejection)
    }

    const CAND_A: &str = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
    const CAND_B: &str = "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222";

    #[tokio::test]
    async fn test_batch_remove_candidates_applies_and_reports_missing() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        // housekeeping_candidates.photo_hash has an FK to photos(hash_sha256)
        // (ON DELETE CASCADE), so the candidate rows need real photo rows.
        for hash in [CAND_A, CAND_B] {
            sqlx::query(
                "INSERT INTO photos (hash_sha256, file_path, filename, file_size, file_modified) \
                 VALUES (?, ?, ?, 1, '2026-01-01 00:00:00')",
            )
            .bind(hash)
            .bind(format!("/tmp/{}.jpg", hash))
            .bind(format!("{}.jpg", hash))
            .execute(&db_pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO housekeeping_candidates (photo_hash, reason, score) VALUES (?, ?, ?)",
            )
            .bind(hash)
            .bind("receipt")
            .bind(95.0f32)
            .execute(&db_pool)
            .await
            .unwrap();
        }
        let routes = build_test_routes(db_pool.clone());
        let missing = "cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333";

        let response = warp::test::request()
            .method("POST")
            .path("/api/housekeeping/candidates/batch-remove")
            .json(&serde_json::json!({"hashes": [CAND_A, CAND_B, missing]}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let result: BatchResult = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(result.applied.len(), 2);
        assert!(result.applied.contains(&CAND_A.to_string()));
        assert!(result.applied.contains(&CAND_B.to_string()));
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].id, missing);
        assert_eq!(result.failed[0].error, "Candidate not found");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping_candidates")
            .fetch_one(&db_pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_batch_remove_candidates_rejects_empty() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let routes = build_test_routes(db_pool);

        let response = warp::test::request()
            .method("POST")
            .path("/api/housekeeping/candidates/batch-remove")
            .json(&serde_json::json!({"hashes": []}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }
}
