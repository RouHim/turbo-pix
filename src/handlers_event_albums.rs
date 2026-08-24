use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use warp::http::StatusCode;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::DbPool;
use crate::event_albums::{self, EventAlbum};
use crate::handlers_photo::{
    PhotosResponse, DEFAULT_PAGE, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, MIN_PAGE_SIZE,
};
use crate::warp_helpers::{with_db, DatabaseError, ValidationError};

const MAX_JSON_BODY_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct EventAlbumsResponse {
    pub event_albums: Vec<EventAlbum>,
}

#[derive(Debug, Deserialize)]
pub struct AlbumRequest {
    pub name: String,
    pub start_date: String,
    pub end_date: String,
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AlbumPhotosQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

struct AlbumFields {
    name: String,
    start_date: String,
    end_date: String,
    location: Option<String>,
}

/// 404 reply matching `handle_rejection`'s not-found shape. Emitted directly
/// (not `reject::not_found()`) for the same reason as the saved-searches route:
/// inside warp's `or`, a NotFound rejection is dropped when a sibling route on
/// the same path rejects with MethodNotAllowed, surfacing as 405 instead of 404.
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

/// Validates and normalizes an album request: non-empty name, `YYYY-MM-DD`
/// dates, `start_date <= end_date`, and an optional trimmed location.
fn validate_album(req: &AlbumRequest) -> Result<AlbumFields, ValidationError> {
    let name = validate_name(&req.name)?;

    let start = NaiveDate::parse_from_str(req.start_date.trim(), "%Y-%m-%d").map_err(|_| {
        ValidationError {
            message: "Invalid start date".to_string(),
        }
    })?;
    let end = NaiveDate::parse_from_str(req.end_date.trim(), "%Y-%m-%d").map_err(|_| {
        ValidationError {
            message: "Invalid end date".to_string(),
        }
    })?;
    if start > end {
        return Err(ValidationError {
            message: "Start date must not be after end date".to_string(),
        });
    }

    let location = req
        .location
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string);

    Ok(AlbumFields {
        name,
        start_date: start.format("%Y-%m-%d").to_string(),
        end_date: end.format("%Y-%m-%d").to_string(),
        location,
    })
}

pub async fn list_event_albums(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match event_albums::list(&db_pool).await {
        Ok(albums) => Ok(warp::reply::json(&EventAlbumsResponse {
            event_albums: albums,
        })),
        Err(e) => {
            log::error!("Failed to list event albums: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to list event albums: {e}"),
            }))
        }
    }
}

pub async fn create_event_album(
    req: AlbumRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let fields = validate_album(&req)?;
    match event_albums::create(
        &db_pool,
        &fields.name,
        &fields.start_date,
        &fields.end_date,
        fields.location.as_deref(),
    )
    .await
    {
        Ok(created) => Ok(warp::reply::with_status(
            warp::reply::json(&created),
            StatusCode::CREATED,
        )),
        Err(e) => {
            log::error!("Failed to create event album: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to create event album: {e}"),
            }))
        }
    }
}

pub async fn update_event_album(
    id: i64,
    req: AlbumRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let fields = validate_album(&req)?;
    match event_albums::update(
        &db_pool,
        id,
        &fields.name,
        &fields.start_date,
        &fields.end_date,
        fields.location.as_deref(),
    )
    .await
    {
        Ok(Some(row)) => Ok(warp::reply::json(&row).into_response()),
        Ok(None) => Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to update event album {id}: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to update event album {id}: {e}"),
            }))
        }
    }
}

pub async fn delete_event_album(id: i64, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match event_albums::delete(&db_pool, id).await {
        Ok(true) => {
            Ok(warp::reply::with_status(warp::reply(), StatusCode::NO_CONTENT).into_response())
        }
        Ok(false) => Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to delete event album {id}: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to delete event album {id}: {e}"),
            }))
        }
    }
}

pub async fn list_album_photos(
    id: i64,
    query: AlbumPhotosQuery,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let album = match event_albums::find_by_id(&db_pool, id).await {
        Ok(Some(album)) => album,
        Ok(None) => return Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to load event album {id}: {e}");
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to load event album {id}: {e}"),
            }));
        }
    };

    let page = query.page.unwrap_or(DEFAULT_PAGE).max(DEFAULT_PAGE);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE);
    let offset = (page as u64 - 1) * limit as u64;

    match event_albums::photos_for_album(
        &db_pool,
        &album,
        limit as i64,
        offset as i64,
        query.sort.as_deref(),
        query.order.as_deref(),
    )
    .await
    {
        Ok((photos, total)) => {
            let has_next = offset.saturating_add(limit as u64) < total as u64;
            let has_prev = page > 1;
            Ok(warp::reply::json(&PhotosResponse {
                photos,
                total: total as usize,
                page,
                limit,
                has_next,
                has_prev,
            })
            .into_response())
        }
        Err(e) => {
            log::error!("Failed to list event album {id} photos: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to list event album {id} photos: {e}"),
            }))
        }
    }
}

pub fn build_event_albums_routes(
    db_pool: DbPool,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    let list = warp::path!("api" / "event-albums")
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(list_event_albums);

    let create = warp::path!("api" / "event-albums")
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<AlbumRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(create_event_album);

    let update = warp::path!("api" / "event-albums" / i64)
        .and(warp::patch())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<AlbumRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(update_event_album);

    let delete = warp::path!("api" / "event-albums" / i64)
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and_then(delete_event_album);

    let photos = warp::path!("api" / "event-albums" / i64 / "photos")
        .and(warp::get())
        .and(warp::query::<AlbumPhotosQuery>())
        .and(with_db(db_pool))
        .and_then(list_album_photos);

    list.or(create).or(update).or(delete).or(photos)
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
        build_event_albums_routes(db_pool).recover(handle_rejection)
    }

    fn body(name: &str, start: &str, end: &str) -> serde_json::Value {
        serde_json::json!({ "name": name, "start_date": start, "end_date": end, "location": "Berlin" })
    }

    #[tokio::test]
    async fn test_create_returns_201_and_lists() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let res = warp::test::request()
            .method("POST")
            .path("/api/event-albums")
            .json(&body("Berlin trip", "2024-01-01", "2024-01-31"))
            .reply(&routes)
            .await;
        assert_eq!(res.status(), StatusCode::CREATED);
        let created: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(created["name"], "Berlin trip");
        assert_eq!(created["location"], "Berlin");
        assert!(created["id"].as_i64().is_some());

        let list = warp::test::request()
            .path("/api/event-albums")
            .reply(&routes)
            .await;
        let listed: serde_json::Value = serde_json::from_slice(list.body()).unwrap();
        assert_eq!(listed["event_albums"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_create_validation_errors() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        for bad in [
            body("  ", "2024-01-01", "2024-01-31"), // empty name
            body("x", "2024-01-31", "2024-01-01"),  // start > end
            body("x", "not-a-date", "2024-01-31"),  // invalid start
        ] {
            let res = warp::test::request()
                .method("POST")
                .path("/api/event-albums")
                .json(&bad)
                .reply(&routes)
                .await;
            assert_eq!(res.status(), StatusCode::BAD_REQUEST, "body: {bad}");
        }
        // nothing persisted
        let list = warp::test::request()
            .path("/api/event-albums")
            .reply(&routes)
            .await;
        let listed: serde_json::Value = serde_json::from_slice(list.body()).unwrap();
        assert_eq!(listed["event_albums"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_update_and_delete_lifecycle() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);
        let created: serde_json::Value = serde_json::from_slice(
            warp::test::request()
                .method("POST")
                .path("/api/event-albums")
                .json(&body("Old", "2024-01-01", "2024-01-31"))
                .reply(&routes)
                .await
                .body(),
        )
        .unwrap();
        let id = created["id"].as_i64().unwrap();

        let updated = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/event-albums/{id}"))
            .json(&serde_json::json!({ "name": "New", "start_date": "2024-02-01", "end_date": "2024-02-29", "location": null }))
            .reply(&routes)
            .await;
        assert_eq!(updated.status(), StatusCode::OK);
        let parsed: serde_json::Value = serde_json::from_slice(updated.body()).unwrap();
        assert_eq!(parsed["name"], "New");
        assert_eq!(parsed["location"], serde_json::Value::Null);

        let missing = warp::test::request()
            .method("PATCH")
            .path("/api/event-albums/9999")
            .json(&body("x", "2024-01-01", "2024-01-31"))
            .reply(&routes)
            .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let deleted = warp::test::request()
            .method("DELETE")
            .path(&format!("/api/event-albums/{id}"))
            .reply(&routes)
            .await;
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_photos_endpoint_404s_for_unknown_album() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);
        let res = warp::test::request()
            .path("/api/event-albums/9999/photos")
            .reply(&routes)
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
