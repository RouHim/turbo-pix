use serde::{Deserialize, Serialize};
use warp::http::StatusCode;
use warp::{reject, Filter, Rejection, Reply};

use crate::albums::{self, Album};
use crate::db::DbPool;
use crate::handlers_photo::{
    PhotosResponse, DEFAULT_PAGE, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, MIN_PAGE_SIZE,
};
use crate::warp_helpers::{with_db, DatabaseError, ValidationError};

const MAX_JSON_BODY_BYTES: u64 = 1024 * 1024;

/// Cap on member add/remove batch size; larger batches are rejected with 400.
const MAX_MEMBERS_PER_REQUEST: usize = 1000;

#[derive(Debug, Serialize)]
pub struct AlbumsResponse {
    pub albums: Vec<Album>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAlbumRequest {
    pub name: String,
    pub initial_hashes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RenameAlbumRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MembersRequest {
    pub hashes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AlbumPhotosQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AddMembersResponse {
    pub added: usize,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct RemoveMembersResponse {
    pub removed: u64,
    pub total: i64,
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

fn validate_member_hashes(hashes: &[String]) -> Result<(), ValidationError> {
    if hashes.len() > MAX_MEMBERS_PER_REQUEST {
        return Err(ValidationError {
            message: format!("Too many hashes (max {MAX_MEMBERS_PER_REQUEST})"),
        });
    }
    Ok(())
}

pub async fn list_albums(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match albums::list(&db_pool).await {
        Ok(albums) => Ok(warp::reply::json(&AlbumsResponse { albums })),
        Err(e) => {
            log::error!("Failed to list albums: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to list albums: {e}"),
            }))
        }
    }
}

pub async fn create_album(
    req: CreateAlbumRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let name = validate_name(&req.name)?;
    let initial = req.initial_hashes.unwrap_or_default();
    validate_member_hashes(&initial)?;
    match albums::create_with_members(&db_pool, &name, &initial).await {
        Ok(created) => Ok(warp::reply::with_status(
            warp::reply::json(&created),
            StatusCode::CREATED,
        )),
        Err(e) => {
            log::error!("Failed to create album: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to create album: {e}"),
            }))
        }
    }
}

pub async fn rename_album(
    id: i64,
    req: RenameAlbumRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    let name = validate_name(&req.name)?;
    match albums::rename(&db_pool, id, &name).await {
        Ok(Some(row)) => Ok(warp::reply::json(&row).into_response()),
        Ok(None) => Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to rename album {id}: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to rename album {id}: {e}"),
            }))
        }
    }
}

pub async fn delete_album(id: i64, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match albums::delete(&db_pool, id).await {
        Ok(true) => {
            Ok(warp::reply::with_status(warp::reply(), StatusCode::NO_CONTENT).into_response())
        }
        Ok(false) => Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to delete album {id}: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to delete album {id}: {e}"),
            }))
        }
    }
}

pub async fn list_album_photos(
    id: i64,
    query: AlbumPhotosQuery,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    match albums::find_by_id(&db_pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to load album {id}: {e}");
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to load album {id}: {e}"),
            }));
        }
    }

    let page = query.page.unwrap_or(DEFAULT_PAGE).max(DEFAULT_PAGE);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE);
    let offset = (page as u64 - 1) * limit as u64;

    match albums::photos_for_album(
        &db_pool,
        id,
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
            log::error!("Failed to list album {id} photos: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to list album {id} photos: {e}"),
            }))
        }
    }
}

pub async fn add_album_members(
    id: i64,
    req: MembersRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    validate_member_hashes(&req.hashes)?;
    match albums::find_by_id(&db_pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to load album {id}: {e}");
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to load album {id}: {e}"),
            }));
        }
    }
    let added = match albums::add_members(&db_pool, id, &req.hashes).await {
        Ok(added) => added,
        Err(e) => {
            log::error!("Failed to add members to album {id}: {e}");
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to add members to album {id}: {e}"),
            }));
        }
    };
    match albums::count_members(&db_pool, id).await {
        Ok(total) => Ok(warp::reply::json(&AddMembersResponse { added, total }).into_response()),
        Err(e) => {
            log::error!("Failed to count album {id} members: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to count album {id} members: {e}"),
            }))
        }
    }
}

pub async fn remove_album_members(
    id: i64,
    req: MembersRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    validate_member_hashes(&req.hashes)?;
    match albums::find_by_id(&db_pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Ok(not_found_reply()),
        Err(e) => {
            log::error!("Failed to load album {id}: {e}");
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to load album {id}: {e}"),
            }));
        }
    }
    let removed = match albums::remove_members(&db_pool, id, &req.hashes).await {
        Ok(removed) => removed,
        Err(e) => {
            log::error!("Failed to remove members from album {id}: {e}");
            return Err(reject::custom(DatabaseError {
                message: format!("Failed to remove members from album {id}: {e}"),
            }));
        }
    };
    match albums::count_members(&db_pool, id).await {
        Ok(total) => {
            Ok(warp::reply::json(&RemoveMembersResponse { removed, total }).into_response())
        }
        Err(e) => {
            log::error!("Failed to count album {id} members: {e}");
            Err(reject::custom(DatabaseError {
                message: format!("Failed to count album {id} members: {e}"),
            }))
        }
    }
}

pub fn build_albums_routes(
    db_pool: DbPool,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    let list = warp::path!("api" / "albums")
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(list_albums);

    let create = warp::path!("api" / "albums")
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<CreateAlbumRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(create_album);

    let rename = warp::path!("api" / "albums" / i64)
        .and(warp::patch())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<RenameAlbumRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(rename_album);

    let delete = warp::path!("api" / "albums" / i64)
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and_then(delete_album);

    let photos = warp::path!("api" / "albums" / i64 / "photos")
        .and(warp::get())
        .and(warp::query::<AlbumPhotosQuery>())
        .and(with_db(db_pool.clone()))
        .and_then(list_album_photos);

    let add = warp::path!("api" / "albums" / i64 / "members")
        .and(warp::post())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<MembersRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(add_album_members);

    let remove = warp::path!("api" / "albums" / i64 / "members")
        .and(warp::delete())
        .and(warp::body::content_length_limit(MAX_JSON_BODY_BYTES))
        .and(warp::body::json::<MembersRequest>())
        .and(with_db(db_pool))
        .and_then(remove_album_members);

    list.or(create)
        .or(rename)
        .or(delete)
        .or(photos)
        .or(add)
        .or(remove)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;
    use crate::db::Photo;
    use crate::db_pool::create_in_memory_pool;
    use crate::warp_helpers::handle_rejection;

    fn build_test_routes(
        db_pool: DbPool,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        build_albums_routes(db_pool).recover(handle_rejection)
    }

    fn seed_photo(photo_hash: &str) -> Photo {
        Photo {
            hash_sha256: photo_hash.to_string(),
            file_path: format!("./test/{photo_hash}.jpg"),
            filename: format!("{photo_hash}.jpg"),
            file_size: 0,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            width: None,
            height: None,
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: None,
            blurhash: None,
            is_favorite: None,
            semantic_vector_indexed: None,
            metadata: serde_json::json!({}),
            date_modified: chrono::Utc::now(),
            date_indexed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    async fn create_album_via_api(db_pool: &DbPool, name: &str) -> serde_json::Value {
        let routes = build_test_routes(db_pool.clone());
        serde_json::from_slice(
            warp::test::request()
                .method("POST")
                .path("/api/albums")
                .json(&serde_json::json!({ "name": name }))
                .reply(&routes)
                .await
                .body(),
        )
        .unwrap()
    }

    // GIVEN an album creation request with an empty name
    // WHEN POST /api/albums is called
    // THEN it rejects with 400 and saves nothing
    #[tokio::test]
    async fn test_create_rejects_empty_name() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let res = warp::test::request()
            .method("POST")
            .path("/api/albums")
            .json(&serde_json::json!({ "name": "  " }))
            .reply(&routes)
            .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let list = warp::test::request()
            .path("/api/albums")
            .reply(&routes)
            .await;
        let listed: serde_json::Value = serde_json::from_slice(list.body()).unwrap();
        assert_eq!(listed["albums"].as_array().unwrap().len(), 0);
    }

    // GIVEN an album with a member added twice
    // WHEN POST /api/albums/{id}/members repeats the hash
    // THEN the second call succeeds with added=0 and total unchanged
    #[tokio::test]
    async fn test_add_member_idempotent() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool.clone());
        let hash = "a".repeat(64);
        seed_photo(&hash).create(&db_pool).await.unwrap();
        let created = create_album_via_api(&db_pool, "A").await;
        let id = created["id"].as_i64().unwrap();

        let first = warp::test::request()
            .method("POST")
            .path(&format!("/api/albums/{id}/members"))
            .json(&serde_json::json!({ "hashes": [hash] }))
            .reply(&routes)
            .await;
        assert_eq!(first.status(), StatusCode::OK);
        let parsed: serde_json::Value = serde_json::from_slice(first.body()).unwrap();
        assert_eq!(parsed["added"], 1);
        assert_eq!(parsed["total"], 1);

        let second = warp::test::request()
            .method("POST")
            .path(&format!("/api/albums/{id}/members"))
            .json(&serde_json::json!({ "hashes": [hash] }))
            .reply(&routes)
            .await;
        assert_eq!(second.status(), StatusCode::OK);
        let parsed: serde_json::Value = serde_json::from_slice(second.body()).unwrap();
        assert_eq!(parsed["added"], 0);
        assert_eq!(parsed["total"], 1);
    }

    // GIVEN a missing album id
    // WHEN GET/PATCH/DELETE/members hits it
    // THEN it returns the shared 404 JSON shape (not 405)
    #[tokio::test]
    async fn test_missing_album_is_404() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let routes = build_test_routes(db_pool);

        let cases = vec![
            ("GET", "/api/albums/9999/photos", None),
            (
                "PATCH",
                "/api/albums/9999",
                Some(serde_json::json!({ "name": "x" })),
            ),
            ("DELETE", "/api/albums/9999", None),
            (
                "POST",
                "/api/albums/9999/members",
                Some(serde_json::json!({ "hashes": [] })),
            ),
        ];
        for (method, path, body) in cases {
            let mut req = warp::test::request().method(method).path(path);
            if let Some(body) = &body {
                req = req.json(body);
            }
            let res = req.reply(&routes).await;
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{method} {path}");
            let parsed: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
            assert_eq!(parsed["code"], 404);
        }

        // DELETE with a body needs its own request (warp test builder consumes).
        let res = warp::test::request()
            .method("DELETE")
            .path("/api/albums/9999/members")
            .json(&serde_json::json!({ "hashes": [] }))
            .reply(&routes)
            .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
