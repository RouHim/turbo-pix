use crate::db::crud::{SearchQuery, SearchSuggestion};
use crate::db::{DbPool, Photo};
use crate::web::handlers::photos::{ErrorResponse, PhotosResponse};
use actix_web::{web, HttpResponse, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SearchSuggestionsResponse {
    pub suggestions: Vec<SearchSuggestion>,
}

pub async fn search_photos(
    pool: web::Data<DbPool>,
    query: web::Query<SearchQuery>,
) -> Result<HttpResponse> {
    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = (page - 1) * limit;

    let _sort_field = query.sort.as_deref().unwrap_or("date_indexed");
    let _sort_order = query.order.as_deref().unwrap_or("desc");

    match Photo::search_photos(&pool, &query, limit as i64, offset as i64) {
        Ok((photos, total)) => {
            let has_next = offset + limit < total as u32;
            let has_prev = page > 1;

            Ok(HttpResponse::Ok().json(PhotosResponse {
                photos,
                total,
                page,
                limit,
                has_next,
                has_prev,
            }))
        }
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Search error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}

pub async fn search_suggestions(
    pool: web::Data<DbPool>,
    query: web::Query<SearchQuery>,
) -> Result<HttpResponse> {
    match Photo::get_search_suggestions(&pool, query.q.as_deref()) {
        Ok(suggestions) => Ok(HttpResponse::Ok().json(SearchSuggestionsResponse { suggestions })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Suggestions error: {}", e),
            code: 500,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })),
    }
}
