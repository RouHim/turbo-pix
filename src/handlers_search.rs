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
    #[serde(default = "default_offset")]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

fn default_offset() -> usize {
    0
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
    log::info!(
        "Semantic search query: '{}' (limit: {}, offset: {})",
        query.q,
        query.limit,
        query.offset
    );

    // Perform semantic search
    let results = semantic_search
        .search(&query.q, query.limit, query.offset)
        .await
        .map_err(|e| {
            log::error!("Semantic search error: {}", e);
            reject::custom(DatabaseError {
                message: format!("Semantic search error: {}", e),
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

    // Query all hashes in a single batch using sqlx
    let paths: Vec<String> = path_scores.keys().cloned().collect();
    
    // Construct dynamic IN query
    let placeholders: Vec<String> = paths.iter().map(|_| "?".to_string()).collect();
    let query_sql = format!(
        "SELECT file_path, hash_sha256 FROM photos WHERE file_path IN ({})",
        placeholders.join(",")
    );

    let mut query_builder = sqlx::query_as::<_, (String, String)>(&query_sql);
    for path in &paths {
        query_builder = query_builder.bind(path);
    }

    let rows = query_builder.fetch_all(&db_pool).await.map_err(|e| {
        log::error!("Database query failed: {}", e);
        reject::custom(DatabaseError {
            message: format!("Database error: {}", e),
        })
    })?;

    let mut search_results: Vec<SemanticSearchResult> = rows
        .into_iter()
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