use serde::{Deserialize, Serialize};
use warp::http::StatusCode;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::DbPool;
use crate::saved_searches::{self, CreateError, SavedSearch, VALID_SORTS, VALID_VIEWS};
use crate::warp_helpers::{with_db, DatabaseError, ValidationError};

/// Cap on JSON request bodies; the one in handlers_photo.rs is private.
const MAX_JSON_BODY_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct SavedSearchesResponse {
    pub saved_searches: Vec<SavedSearch>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSavedSearchRequest {
    pub name: String,
    pub query: Option<String>,
    pub view: String,
    pub sort: String,
    pub year: Option<i64>,
    pub month: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RenameSavedSearchRequest {
    pub name: String,
}

type CreateFields = (
    String,
    Option<String>,
    String,
    String,
    Option<i64>,
    Option<i64>,
);

/// Generic 404 reply matching `handle_rejection`'s not-found shape.
///
/// NOT a `reject::not_found()`: in warp's `or` combinator, a `NotFound`
/// rejection is silently dropped when a sibling route on the same path
/// rejects with `MethodNotAllowed` (reject.rs `combine` ignores NotFound),
/// so a handler-level not-found would surface as 405. Because this route
/// already matched, replying directly is the only way to produce a real 404.
fn not_found_reply() -> warp::reply::Response {
    let timestamp = chrono::Utc::now().to_rfc3339();
    warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Not Found",
            "code": 404,
            "timestamp": timestamp,
        })),
        StatusCode::NOT_FOUND,
    )
    .into_response()
}

fn validate_name(name: &str) -> Result<String, ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ValidationError {
            message: "Name is required".to_string(),
        });
    }
    if trimmed.len() > 200 {
        return Err(ValidationError {
            message: "Name too long".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

fn validate_create(req: &CreateSavedSearchRequest) -> Result<CreateFields, ValidationError> {
    let name = validate_name(&req.name)?;
    let query = req
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string);

    if !VALID_VIEWS.contains(&req.view.as_str()) {
        return Err(ValidationError {
            message: "Invalid view".to_string(),
        });
    }
    if !VALID_SORTS.contains(&req.sort.as_str()) {
        return Err(ValidationError {
            message: "Invalid sort".to_string(),
        });
    }
    if req.year.is_some_and(|y| y < 1) {
        return Err(ValidationError {
            message: "Invalid year".to_string(),
        });
    }
    // Mirrors router normalizeState: a month requires a year.
    if req.month.is_some_and(|m| !(1..=12).contains(&m))
        || (req.month.is_some() && req.year.is_none())
    {
        return Err(ValidationError {
            message: "Invalid month".to_string(),
        });
    }

    Ok((
        name,
        query,
        req.view.clone(),
        req.sort.clone(),
        req.year,
        req.month,
    ))
}

pub async fn list_saved_searches(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match saved_searches::list(&db_pool).await {
        Ok(saved_searches) => Ok(warp::reply::json(&SavedSearchesResponse { saved_searches })),
        Err(e) => {
            log::error!("Failed to list saved searches: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to list saved searches: {}", e),
            }))
        }
    }
}

pub async fn create_saved_search(
    req: CreateSavedSearchRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let (name, query, view, sort, year, month) = validate_create(&req)?;

    match saved_searches::create(&db_pool, &name, query.as_deref(), &view, &sort, year, month).await
    {
        Ok(created) => Ok(warp::reply::with_status(
            warp::reply::json(&created),
            StatusCode::CREATED,
        )),
        Err(CreateError::Duplicate(existing)) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({
                "error": "already_saved",
                "saved_search": existing,
            })),
            StatusCode::CONFLICT,
        )),
        Err(CreateError::Db(e)) => {
            log::error!("Failed to create saved search: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to create saved search: {}", e),
            }))
        }
    }
}

pub async fn rename_saved_search(
    id: i64,
    req: RenameSavedSearchRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let name = validate_name(&req.name)?;

    match saved_searches::rename(&db_pool, id, &name).await {
        Ok(Some(row)) => Ok(warp::reply::json(&row).into_response()),
        Ok(None) => Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to rename saved search {}: {}", id, e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to rename saved search: {}", e),
            }))
        }
    }
}

pub async fn delete_saved_search(id: i64, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match saved_searches::delete(&db_pool, id).await {
        Ok(true) => {
            Ok(warp::reply::with_status(warp::reply(), StatusCode::NO_CONTENT).into_response())
        }
        Ok(false) => Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to delete saved search {}: {}", id, e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to delete saved search: {}", e),
            }))
        }
    }
}

pub fn build_saved_searches_routes(
    db_pool: DbPool,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    let list = warp::path!("api" / "saved-searches")
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(list_saved_searches);

    let create = warp::path!("api" / "saved-searches")
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<CreateSavedSearchRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(create_saved_search);

    let rename = warp::path!("api" / "saved-searches" / i64)
        .and(warp::patch())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<RenameSavedSearchRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(rename_saved_search);

    let delete = warp::path!("api" / "saved-searches" / i64)
        .and(warp::delete())
        .and(with_db(db_pool))
        .and_then(delete_saved_search);

    list.or(create).or(rename).or(delete)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use crate::db_pool::create_in_memory_pool;
    use crate::warp_helpers::handle_rejection;

    fn build_test_routes(
        db_pool: DbPool,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        build_saved_searches_routes(db_pool).recover(handle_rejection)
    }

    fn create_body(name: &str) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "query": "beach",
            "view": "all",
            "sort": "date_desc",
            "year": 2023,
            "month": null,
        })
    }

    #[tokio::test]
    async fn test_create_returns_201_with_created_row() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let response = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&create_body("Beach 2023"))
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["name"], "Beach 2023");
        assert_eq!(body["query"], "beach");
        assert_eq!(body["view"], "all");
        assert_eq!(body["sort"], "date_desc");
        assert_eq!(body["year"], 2023);
        assert_eq!(body["month"], serde_json::Value::Null);
        assert!(body["id"].as_i64().is_some());
    }

    #[tokio::test]
    async fn test_create_null_query_round_trips() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let mut body = create_body("Null query");
        body["query"] = serde_json::Value::Null;
        let response = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&body)
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let parsed: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(parsed["query"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_create_duplicate_returns_409() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let first = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&create_body("Beach 2023"))
            .reply(&routes)
            .await;
        assert_eq!(first.status(), StatusCode::CREATED);

        let second = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&create_body("Beach 2023"))
            .reply(&routes)
            .await;
        assert_eq!(second.status(), StatusCode::CONFLICT);
        let body: serde_json::Value = serde_json::from_slice(second.body()).unwrap();
        assert_eq!(body["error"], "already_saved");
        assert_eq!(body["saved_search"]["name"], "Beach 2023");
    }

    #[tokio::test]
    async fn test_create_invalid_view_returns_400() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let mut body = create_body("Bad view");
        body["view"] = serde_json::json!("unknown");
        let response = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&body)
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_invalid_sort_returns_400() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let mut body = create_body("Bad sort");
        body["sort"] = serde_json::json!("random");
        let response = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&body)
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_empty_name_returns_400() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let mut body = create_body("   ");
        body["name"] = serde_json::json!("   ");
        let response = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&body)
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_month_without_year_returns_400() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let mut body = create_body("Month no year");
        body["year"] = serde_json::Value::Null;
        body["month"] = serde_json::json!(7);
        let response = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&body)
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_zero_year_returns_400() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let mut body = create_body("Zero year");
        body["year"] = serde_json::json!(0);
        let response = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&body)
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_returns_wrapper() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool.clone());

        warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&create_body("Beach 2023"))
            .reply(&routes)
            .await;

        let response = warp::test::request()
            .method("GET")
            .path("/api/saved-searches")
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["saved_searches"].as_array().unwrap().len(), 1);
        assert_eq!(body["saved_searches"][0]["name"], "Beach 2023");
    }

    #[tokio::test]
    async fn test_rename_returns_200_with_new_name() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool.clone());

        let created = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&create_body("Old"))
            .reply(&routes)
            .await;
        let id: serde_json::Value = serde_json::from_slice(created.body()).unwrap();
        let id = id["id"].as_i64().unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/saved-searches/{}", id))
            .json(&serde_json::json!({ "name": "New" }))
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["name"], "New");
    }

    #[tokio::test]
    async fn test_rename_empty_name_returns_400() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool.clone());

        let created = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&create_body("Old"))
            .reply(&routes)
            .await;
        let id: serde_json::Value = serde_json::from_slice(created.body()).unwrap();
        let id = id["id"].as_i64().unwrap();

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/saved-searches/{}", id))
            .json(&serde_json::json!({ "name": "   " }))
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_rename_missing_id_returns_404() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let response = warp::test::request()
            .method("PATCH")
            .path("/api/saved-searches/999")
            .json(&serde_json::json!({ "name": "New" }))
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_returns_204() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool.clone());

        let created = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&create_body("Temp"))
            .reply(&routes)
            .await;
        let id: serde_json::Value = serde_json::from_slice(created.body()).unwrap();
        let id = id["id"].as_i64().unwrap();

        let response = warp::test::request()
            .method("DELETE")
            .path(&format!("/api/saved-searches/{}", id))
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(response.body().is_empty());
    }

    #[tokio::test]
    async fn test_delete_missing_id_returns_404() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let response = warp::test::request()
            .method("DELETE")
            .path("/api/saved-searches/999")
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
