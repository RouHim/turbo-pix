use rusqlite::params;
use serde::Serialize;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::{DbPool, Photo};
use crate::warp_helpers::{with_db, DatabaseError, NotFoundError};

#[derive(Debug, Serialize)]
pub struct CleanupCandidate {
    pub photo: Photo,
    pub reason: String,
    pub score: f32,
}

#[derive(Debug, Serialize)]
pub struct CleanupResponse {
    pub candidates: Vec<CleanupCandidate>,
}

pub async fn list_cleanup_candidates(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    let conn = db_pool.get().map_err(|e| {
        reject::custom(DatabaseError {
            message: format!("DB connection error: {}", e),
        })
    })?;

    let mut stmt = conn
        .prepare(
            r#"
        SELECT 
            p.hash_sha256, p.file_path, p.filename, p.file_size, p.mime_type, 
            p.taken_at, p.width, p.height, p.orientation, p.duration, 
            p.thumbnail_path, p.has_thumbnail, p.blurhash, p.is_favorite, 
            p.semantic_vector_indexed, p.metadata, p.file_modified, p.date_indexed, 
            p.created_at, p.updated_at,
            c.reason, c.score
        FROM cleanup_candidates c
        JOIN photos p ON c.photo_hash = p.hash_sha256
        ORDER BY c.score DESC
        LIMIT 100
        "#,
        )
        .map_err(|e| {
            reject::custom(DatabaseError {
                message: format!("Failed to prepare statement: {}", e),
            })
        })?;

    let candidate_iter = stmt
        .query_map([], |row| {
            let photo = Photo::from_row(row)?;
            let reason: String = row.get(20)?;
            let score: f32 = row.get(21)?;

            Ok(CleanupCandidate {
                photo,
                reason,
                score,
            })
        })
        .map_err(|e| {
            reject::custom(DatabaseError {
                message: format!("Query execution failed: {}", e),
            })
        })?;

    let mut candidates = Vec::new();
    for candidate in candidate_iter {
        match candidate {
            Ok(c) => candidates.push(c),
            Err(e) => log::error!("Error parsing cleanup candidate: {}", e),
        }
    }

    Ok(warp::reply::json(&CleanupResponse { candidates }))
}

pub async fn remove_cleanup_candidate(
    hash: String,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let conn = db_pool.get().map_err(|e| {
        reject::custom(DatabaseError {
            message: format!("DB connection error: {}", e),
        })
    })?;

    let affected = conn
        .execute(
            "DELETE FROM cleanup_candidates WHERE photo_hash = ?",
            params![hash],
        )
        .map_err(|e| {
            reject::custom(DatabaseError {
                message: format!("Failed to delete candidate: {}", e),
            })
        })?;

    if affected == 0 {
        return Err(reject::custom(NotFoundError));
    }

    Ok(warp::reply::json(&serde_json::json!({ "success": true })))
}

pub fn build_cleanup_routes(
    db_pool: DbPool,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let list_route = warp::path("api")
        .and(warp::path("cleanup"))
        .and(warp::path("candidates"))
        .and(warp::path::end())
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(list_cleanup_candidates);

    let remove_route = warp::path("api")
        .and(warp::path("cleanup"))
        .and(warp::path("candidates"))
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and_then(remove_cleanup_candidate);

    list_route.or(remove_route)
}
