use serde::Serialize;
use std::sync::Arc;
use warp::{reject, Rejection, Reply};

use crate::clip_encoder::ClipEncoder;
use crate::db::{DbPool, Photo, SearchQuery, SearchSuggestion};
use crate::warp_helpers::DatabaseError;

#[derive(Debug, Serialize)]
pub struct PhotosResponse {
    pub photos: Vec<Photo>,
    pub total: usize,
    pub page: u32,
    pub limit: u32,
    pub has_next: bool,
    pub has_prev: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchSuggestionsResponse {
    pub suggestions: Vec<SearchSuggestion>,
}

#[allow(dead_code)]
pub async fn search_photos(query: SearchQuery, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = (page - 1) * limit;

    let sort_field = query.sort.as_deref().unwrap_or("taken_at");
    let sort_order = query.order.as_deref().unwrap_or("desc");

    match Photo::search_photos(
        &db_pool,
        &query,
        limit as i64,
        offset as i64,
        Some(sort_field),
        Some(sort_order),
    ) {
        Ok((photos, total)) => {
            let has_next = offset + limit < total as u32;
            let has_prev = page > 1;

            Ok(warp::reply::json(&PhotosResponse {
                photos,
                total: total as usize,
                page,
                limit,
                has_next,
                has_prev,
            }))
        }
        Err(e) => {
            log::error!("Search error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Search error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn search_suggestions(
    query: SearchQuery,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    match Photo::get_search_suggestions(&db_pool, query.q.as_deref()) {
        Ok(suggestions) => Ok(warp::reply::json(&SearchSuggestionsResponse {
            suggestions,
        })),
        Err(e) => {
            log::error!("Suggestions error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Suggestions error: {}", e),
            }))
        }
    }
}

#[allow(dead_code)]
pub async fn get_cameras(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Photo::get_cameras(&db_pool) {
        Ok(cameras) => Ok(warp::reply::json(&cameras)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

/// CLIP-based semantic search handler
#[allow(dead_code)]
pub async fn search_photos_clip(
    query: SearchQuery,
    db_pool: DbPool,
    clip_encoder: Option<Arc<tokio::sync::Mutex<ClipEncoder>>>,
) -> Result<impl Reply, Rejection> {
    // Check if CLIP is enabled
    let encoder = clip_encoder.ok_or_else(|| {
        log::error!("CLIP search requested but CLIP is not enabled");
        reject::custom(DatabaseError {
            message: "CLIP search is not enabled. Set CLIP_ENABLE=true and provide CLIP model files.".to_string(),
        })
    })?;

    if let Some(ref q) = query.q {
        let limit = query.limit.unwrap_or(50).min(100);
        let similarity_threshold = 0.7; // Can be made configurable

        // Generate text embedding for the query
        let mut enc = encoder.lock().await;
        let query_embedding = enc
            .encode_text(q)
            .map_err(|e| {
                log::error!("Failed to encode query '{}': {}", q, e);
                reject::custom(DatabaseError {
                    message: format!("Failed to encode query: {}", e),
                })
            })?;
        drop(enc); // Release lock

        // Search using vector similarity
        match crate::db::search_by_clip_embedding(&db_pool, &query_embedding, limit as i64, similarity_threshold) {
            Ok(photos) => Ok(warp::reply::json(&PhotosResponse {
                photos: photos.clone(),
                total: photos.len(),
                page: 1,
                limit,
                has_next: false,
                has_prev: false,
            })),
            Err(e) => {
                log::error!("CLIP search error: {}", e);
                Err(reject::custom(DatabaseError {
                    message: format!("CLIP search error: {}", e),
                }))
            }
        }
    } else {
        Err(reject::custom(DatabaseError {
            message: "No search query provided".to_string(),
        }))
    }
}
