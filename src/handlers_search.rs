use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::DbPool;
use crate::semantic_search::SemanticSearchEngine;
use crate::warp_helpers::{with_db, with_semantic_search, DatabaseError};

#[derive(Debug, Deserialize)]
pub struct SemanticSearchQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Serialize)]
pub struct SemanticSearchResult {
    pub path: String,
    pub hash: String,
    pub score: f32,
}

#[derive(Debug, Serialize)]
pub struct SemanticSearchResponse {
    pub results: Vec<SemanticSearchResult>,
    pub query: String,
    pub total: usize,
}

pub async fn semantic_search(
    query: SemanticSearchQuery,
    db_pool: DbPool,
    semantic_search: Arc<SemanticSearchEngine>,
) -> Result<impl Reply, Rejection> {
    log::info!("Semantic search query: '{}'", query.q);

    // Perform semantic search
    let results = semantic_search.search(&query.q, query.limit).map_err(|e| {
        log::error!("Semantic search error: {}", e);
        reject::custom(DatabaseError {
            message: format!("Semantic search error: {}", e),
        })
    })?;

    // Convert file paths to hashes by looking up in database
    // Use a single query with IN clause to avoid N+1 problem
    let conn = db_pool.get().map_err(|e| {
        log::error!("Database connection error: {}", e);
        reject::custom(DatabaseError {
            message: format!("Database error: {}", e),
        })
    })?;

    if results.is_empty() {
        return Ok(warp::reply::json(&SemanticSearchResponse {
            total: 0,
            query: query.q,
            results: vec![],
        }));
    }

    // Build path-to-score map
    let path_scores: std::collections::HashMap<String, f32> = results.into_iter().collect();

    // Query all hashes in a single batch
    let paths: Vec<&str> = path_scores.keys().map(|s| s.as_str()).collect();
    let placeholders = paths.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query_sql = format!(
        "SELECT file_path, hash_sha256 FROM photos WHERE file_path IN ({})",
        placeholders
    );

    let mut stmt = conn.prepare(&query_sql).map_err(|e| {
        log::error!("Failed to prepare query: {}", e);
        reject::custom(DatabaseError {
            message: format!("Database error: {}", e),
        })
    })?;

    let mut search_results: Vec<SemanticSearchResult> = stmt
        .query_map(rusqlite::params_from_iter(paths), |row| {
            let path: String = row.get(0)?;
            let hash: String = row.get(1)?;
            Ok((path, hash))
        })
        .map_err(|e| {
            log::error!("Query execution failed: {}", e);
            reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            })
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(path, hash)| {
            path_scores
                .get(&path)
                .map(|&score| SemanticSearchResult { path, hash, score })
        })
        .collect();

    search_results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let response = SemanticSearchResponse {
        total: search_results.len(),
        query: query.q,
        results: search_results,
    };

    Ok(warp::reply::json(&response))
}

pub fn build_search_routes(
    db_pool: DbPool,
    semantic_search_engine: Arc<SemanticSearchEngine>,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("api")
        .and(warp::path("search"))
        .and(warp::path("semantic"))
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::query::<SemanticSearchQuery>())
        .and(with_db(db_pool))
        .and(with_semantic_search(semantic_search_engine))
        .and_then(semantic_search)
}
