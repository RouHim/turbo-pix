use serde::{Deserialize, Serialize};
use warp::http::StatusCode;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::DbPool;
use crate::saved_searches::{
    self, CreateError, SavedSearch, SavedSearchFilter, VALID_SORTS, VALID_VIEWS,
};
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
    pub to_year: Option<i64>,
    pub to_month: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RenameSavedSearchRequest {
    pub name: String,
}

type CreateFields = (String, Option<String>, String, String, SavedSearchFilter);

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
    if req.to_year.is_some_and(|y| y < 1) {
        return Err(ValidationError {
            message: "Invalid end year".to_string(),
        });
    }
    if req.to_month.is_some_and(|m| !(1..=12).contains(&m))
        || (req.to_month.is_some() && req.to_year.is_none())
    {
        return Err(ValidationError {
            message: "Invalid end month".to_string(),
        });
    }

    let mut filter = SavedSearchFilter {
        year: req.year,
        month: req.month,
        to_year: req.to_year,
        to_month: req.to_month,
    };
    // Mirrors router normalizeState: reversed bounds become ascending ones.
    // Checked arithmetic: a year whose month index overflows i64 is not a valid
    // calendar bound for this app, and wrapping would misjudge `end < start`
    // (persisting a reversed range) or panic in debug builds.
    if let (Some(year), Some(to_year)) = (filter.year, filter.to_year) {
        let start = year
            .checked_mul(12)
            .and_then(|index| index.checked_add(filter.month.unwrap_or(1) - 1));
        let end = to_year
            .checked_mul(12)
            .and_then(|index| index.checked_add(filter.to_month.unwrap_or(12) - 1));
        let (Some(start), Some(end)) = (start, end) else {
            return Err(ValidationError {
                message: "Invalid year range".to_string(),
            });
        };
        if end < start {
            filter = SavedSearchFilter {
                year: Some(to_year),
                month: filter.to_month,
                to_year: Some(year),
                to_month: filter.month,
            };
        }
    }

    Ok((name, query, req.view.clone(), req.sort.clone(), filter))
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
    let (name, query, view, sort, filter) = validate_create(&req)?;

    match saved_searches::create(&db_pool, &name, query.as_deref(), &view, &sort, &filter).await {
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
        assert_eq!(body["to_year"], serde_json::Value::Null);
        assert_eq!(body["to_month"], serde_json::Value::Null);
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
    async fn test_create_rejects_invalid_range_bounds() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        for (year, to_year, to_month) in [
            (2015, Some(2012), Some(13)),
            (2015, Some(2012), Some(0)),
            (2015, None, Some(3)),
            (2015, Some(0), None),
            (i64::MAX, Some(1), None),
            (2015, Some(i64::MAX), None),
        ] {
            let request = CreateSavedSearchRequest {
                name: "Range".to_string(),
                query: None,
                view: "all".to_string(),
                sort: "date_desc".to_string(),
                year: Some(year),
                month: Some(8),
                to_year,
                to_month,
            };
            let result = validate_create(&request);
            assert!(
                result.is_err(),
                "({year}, {to_year:?}, {to_month:?}) must be rejected"
            );

            // The rejection must reach the wire as 400: the swap arithmetic is
            // checked, so an overflowing year cannot panic the handler task or
            // wrap into a persisted reversed range.
            let body = serde_json::json!({
                "name": "Range",
                "query": serde_json::Value::Null,
                "view": "all",
                "sort": "date_desc",
                "year": year,
                "month": 8,
                "to_year": to_year,
                "to_month": to_month,
            });
            let response = warp::test::request()
                .method("POST")
                .path("/api/saved-searches")
                .json(&body)
                .reply(&routes)
                .await;
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "({year}, {to_year:?}, {to_month:?}) must answer 400"
            );
        }
    }

    #[tokio::test]
    async fn test_create_stores_range_bounds() {
        let pool = create_in_memory_pool().await.unwrap();
        let row = saved_searches::create(
            &pool,
            "Summer",
            None,
            "all",
            "date_desc",
            &saved_searches::SavedSearchFilter {
                year: Some(2012),
                month: Some(3),
                to_year: Some(2015),
                to_month: Some(8),
            },
        )
        .await
        .unwrap();

        assert_eq!(row.year, Some(2012));
        assert_eq!(row.month, Some(3));
        assert_eq!(row.to_year, Some(2015));
        assert_eq!(row.to_month, Some(8));

        // A different range is a different saved search, not a duplicate.
        let other = saved_searches::create(
            &pool,
            "Autumn",
            None,
            "all",
            "date_desc",
            &saved_searches::SavedSearchFilter {
                year: Some(2012),
                month: Some(3),
                to_year: Some(2015),
                to_month: Some(9),
            },
        )
        .await;
        assert!(
            other.is_ok(),
            "distinct ranges must not collide on the unique index"
        );

        // Re-sending the exact range hits the conflict lookup: it must match on
        // the bounds too, otherwise the lookup misses and the handler answers
        // 500 ("vanished") instead of 409 with the existing row.
        let duplicate = saved_searches::create(
            &pool,
            "Summer again",
            None,
            "all",
            "date_desc",
            &saved_searches::SavedSearchFilter {
                year: Some(2012),
                month: Some(3),
                to_year: Some(2015),
                to_month: Some(8),
            },
        )
        .await;
        match duplicate {
            Err(saved_searches::CreateError::Duplicate(existing)) => {
                assert_eq!(existing.id, row.id, "the lookup must find the exact range");
            }
            Err(other) => panic!("expected Duplicate for the same range, got {:?}", other),
            Ok(_) => panic!("expected Duplicate for the same range, got Ok"),
        }
    }

    #[tokio::test]
    async fn test_create_and_list_return_range_bounds() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let mut body = create_body("Summer range");
        body["year"] = serde_json::json!(2012);
        body["month"] = serde_json::json!(3);
        body["to_year"] = serde_json::json!(2015);
        body["to_month"] = serde_json::json!(8);

        let created = warp::test::request()
            .method("POST")
            .path("/api/saved-searches")
            .json(&body)
            .reply(&routes)
            .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let created: serde_json::Value = serde_json::from_slice(created.body()).unwrap();
        assert_eq!(created["year"], 2012);
        assert_eq!(created["month"], 3);
        assert_eq!(created["to_year"], 2015);
        assert_eq!(created["to_month"], 8);

        // The GET path is what a client restores a range from.
        let listed = warp::test::request()
            .method("GET")
            .path("/api/saved-searches")
            .reply(&routes)
            .await;
        assert_eq!(listed.status(), StatusCode::OK);
        let listed: serde_json::Value = serde_json::from_slice(listed.body()).unwrap();
        assert_eq!(listed["saved_searches"][0]["to_year"], 2015);
        assert_eq!(listed["saved_searches"][0]["to_month"], 8);
    }

    #[tokio::test]
    async fn test_create_swaps_reversed_range_bounds() {
        let request = CreateSavedSearchRequest {
            name: "Reversed".to_string(),
            query: None,
            view: "all".to_string(),
            sort: "date_desc".to_string(),
            year: Some(2015),
            month: Some(8),
            to_year: Some(2012),
            to_month: Some(3),
        };
        let (_, _, _, _, filter) = validate_create(&request).unwrap();
        assert_eq!(
            filter,
            SavedSearchFilter {
                year: Some(2012),
                month: Some(3),
                to_year: Some(2015),
                to_month: Some(8),
            }
        );
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
