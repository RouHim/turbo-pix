//! People API: the clustered person cards and their cover crops.

use std::collections::HashSet;

use log::error;
use serde::{Deserialize, Serialize};
use warp::http::StatusCode;
use warp::{reject, Filter, Rejection, Reply};

use crate::cache_manager::CacheManager;
use crate::db::DbPool;
use crate::face_recognition::store::{self, MergeSlot, Person};
use crate::warp_helpers::{not_found_reply, with_cache, with_db, DatabaseError, ValidationError};

const MAX_NAME_CHARS: usize = 100;
/// Keeper plus source persons in one merge.
const MAX_MERGE_PERSONS: usize = 21;
/// Upper bound for one bulk visibility request; the People list is unpaginated
/// but a single request must not carry an arbitrary payload.
const MAX_BULK_PEOPLE: usize = 1000;

#[derive(Debug, Deserialize)]
pub struct PeopleQuery {
    #[serde(default)]
    pub include_hidden: bool,
}

#[derive(Debug, Serialize)]
pub struct PeopleResponse {
    pub people: Vec<Person>,
    /// The merge that can still be undone, if any.
    pub last_merge: Option<MergeSlot>,
}

#[derive(Debug, Deserialize)]
pub struct MergePeopleRequest {
    pub target_id: i64,
    pub source_ids: Vec<i64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hidden: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct MergePeopleResponse {
    pub moved_faces: u64,
    pub removed_people: u64,
    pub undo_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct UndoMergeRequest {
    pub undo_id: i64,
}

#[derive(Debug, Serialize)]
pub struct UndoMergeResponse {
    pub restored_faces: u64,
    pub total_faces: u64,
    pub restored_people: u64,
}

#[derive(Debug, Deserialize)]
pub struct BulkHideRequest {
    pub ids: Vec<i64>,
    pub hidden: bool,
}

#[derive(Debug, Serialize)]
pub struct BulkHideResponse {
    pub updated: u64,
}

#[derive(Debug, Deserialize)]
pub struct PersonUpdate {
    pub name: Option<String>,
    pub hidden: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PersonResponse {
    pub person: store::PersonRow,
}

pub async fn list_people(query: PeopleQuery, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    let people = store::people(&db_pool, query.include_hidden)
        .await
        .map_err(|error| {
            error!("Failed to list people: {error}");
            reject::custom(DatabaseError {
                message: error.to_string(),
            })
        })?;
    let last_merge = store::last_merge(&db_pool).await.map_err(|error| {
        error!("Failed to read the merge undo slot: {error}");
        reject::custom(DatabaseError {
            message: error.to_string(),
        })
    })?;
    Ok(warp::reply::json(&PeopleResponse { people, last_merge }))
}

fn validation_error(message: String) -> Rejection {
    reject::custom(ValidationError { message })
}

/// Trims and validates a person name; `Some(None)` clears the name.
fn validated_name(name: Option<String>) -> Result<Option<Option<String>>, Rejection> {
    let Some(name) = name else {
        return Ok(None);
    };
    let trimmed = name.trim();
    let value = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
    if value
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_NAME_CHARS)
    {
        return Err(validation_error(format!(
            "Person name must be at most {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(Some(value))
}

pub async fn update_person(
    id: i64,
    update: PersonUpdate,
    db_pool: DbPool,
) -> Result<warp::reply::Response, Rejection> {
    let name = validated_name(update.name)?;
    if name.is_none() && update.hidden.is_none() {
        return Err(validation_error("No person fields supplied".to_string()));
    }

    let person = store::update_person(&db_pool, id, name, update.hidden)
        .await
        .map_err(|error| {
            error!("Failed to update person {id}: {error}");
            reject::custom(DatabaseError {
                message: error.to_string(),
            })
        })?;
    match person {
        Some(person) => Ok(warp::reply::json(&PersonResponse { person }).into_response()),
        None => Ok(not_found_reply()),
    }
}

pub async fn merge_people(
    request: MergePeopleRequest,
    db_pool: DbPool,
) -> Result<warp::reply::Response, Rejection> {
    if request.source_ids.is_empty() || request.source_ids.len() + 1 > MAX_MERGE_PERSONS {
        return Err(validation_error(format!(
            "Merge requires between 2 and {MAX_MERGE_PERSONS} persons"
        )));
    }
    let mut seen = HashSet::with_capacity(request.source_ids.len() + 1);
    if !seen.insert(request.target_id) || !request.source_ids.iter().all(|id| seen.insert(*id)) {
        return Err(validation_error(
            "Merge selection contains duplicate persons".to_string(),
        ));
    }
    let name = validated_name(request.name)?;

    let outcome = store::merge_people(
        &db_pool,
        request.target_id,
        &request.source_ids,
        name,
        request.hidden,
    )
    .await
    .map_err(|error| {
        error!(
            "Failed to merge persons into {}: {error}",
            request.target_id
        );
        reject::custom(DatabaseError {
            message: error.to_string(),
        })
    })?;
    let Some(outcome) = outcome else {
        return Ok(not_found_reply());
    };
    Ok(warp::reply::json(&MergePeopleResponse {
        moved_faces: outcome.moved_faces,
        removed_people: outcome.removed_people,
        undo_id: outcome.undo_id,
    })
    .into_response())
}

pub async fn undo_merge(
    request: UndoMergeRequest,
    db_pool: DbPool,
) -> Result<warp::reply::Response, Rejection> {
    match store::undo_merge(&db_pool, request.undo_id).await {
        Ok(store::UndoResult::Done(outcome)) => Ok(warp::reply::json(&UndoMergeResponse {
            restored_faces: outcome.restored_faces,
            total_faces: outcome.total_faces,
            restored_people: outcome.restored_people,
        })
        .into_response()),
        Ok(store::UndoResult::Missing) => Ok(not_found_reply()),
        Ok(store::UndoResult::Stale) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({ "error": "merge_slot_stale" })),
            StatusCode::CONFLICT,
        )
        .into_response()),
        Err(error) => {
            error!("Failed to undo merge {}: {error}", request.undo_id);
            Err(reject::custom(DatabaseError {
                message: error.to_string(),
            }))
        }
    }
}

pub async fn bulk_hide_people(
    request: BulkHideRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    if request.ids.is_empty() || request.ids.len() > MAX_BULK_PEOPLE {
        return Err(validation_error(format!(
            "Between 1 and {MAX_BULK_PEOPLE} person ids are required"
        )));
    }
    let updated = store::set_people_hidden(&db_pool, &request.ids, request.hidden)
        .await
        .map_err(|error| {
            error!("Failed to update person visibility: {error}");
            reject::custom(DatabaseError {
                message: error.to_string(),
            })
        })?;
    Ok(warp::reply::json(&BulkHideResponse { updated }))
}

pub async fn face_crop(
    face_id: i64,
    db_pool: DbPool,
    cache_manager: CacheManager,
) -> Result<impl Reply, Rejection> {
    let Some(hash) = store::face_crop_info(&db_pool, face_id)
        .await
        .map_err(|error| {
            error!("Failed to resolve face {face_id}: {error}");
            reject::custom(DatabaseError {
                message: error.to_string(),
            })
        })?
    else {
        return Ok(not_found_reply());
    };

    let path = cache_manager.face_crop_path(&hash, face_id);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(not_found_reply()),
        Err(error) => {
            error!("Failed to read face crop {}: {error}", path.display());
            return Err(reject::custom(DatabaseError {
                message: error.to_string(),
            }));
        }
    };

    Ok(warp::reply::with_header(
        warp::reply::with_header(bytes, "content-type", "image/jpeg"),
        "cache-control",
        "public, max-age=31536000, immutable",
    )
    .into_response())
}

pub fn build_people_routes(
    db_pool: DbPool,
    cache_manager: CacheManager,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let list = warp::path!("api" / "people")
        .and(warp::get())
        .and(warp::query::<PeopleQuery>())
        .and(with_db(db_pool.clone()))
        .and_then(list_people);

    // Literal sub-paths are registered before the parameterized PATCH route:
    // `/api/people/merge` must never be read as a person id.
    let merge = warp::path!("api" / "people" / "merge")
        .and(warp::post())
        .and(warp::body::content_length_limit(1024 * 1024))
        .and(warp::body::json())
        .and(with_db(db_pool.clone()))
        .and_then(merge_people);

    let undo_merge = warp::path!("api" / "people" / "merge" / "undo")
        .and(warp::post())
        .and(warp::body::content_length_limit(1024 * 1024))
        .and(warp::body::json())
        .and(with_db(db_pool.clone()))
        .and_then(undo_merge);

    let bulk_hide = warp::path!("api" / "people" / "batch" / "hide")
        .and(warp::post())
        .and(warp::body::content_length_limit(1024 * 1024))
        .and(warp::body::json())
        .and(with_db(db_pool.clone()))
        .and_then(bulk_hide_people);

    let update = warp::path!("api" / "people" / i64)
        .and(warp::patch())
        .and(warp::body::content_length_limit(1024 * 1024))
        .and(warp::body::json())
        .and(with_db(db_pool.clone()))
        .and_then(update_person);

    let crop = warp::path!("api" / "faces" / i64 / "crop")
        .and(warp::get())
        .and(with_db(db_pool))
        .and(with_cache(cache_manager))
        .and_then(face_crop);

    list.or(merge)
        .or(undo_merge)
        .or(bulk_hide)
        .or(update)
        .or(crop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_in_memory_pool;
    use crate::db::tests::create_test_photo;
    use crate::warp_helpers::handle_rejection;

    async fn seed_person(pool: &DbPool, name: &str, hidden: bool, faces: i64) -> i64 {
        let id: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
            .fetch_one(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE people SET name = ?, hidden = ? WHERE id = ?")
            .bind(name)
            .bind(hidden)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        let photo = create_test_photo(format!("{id}.jpg"), format!("{id}"));
        photo.create(pool).await.unwrap();
        for index in 0..faces {
            sqlx::query(
                "INSERT INTO faces
                 (photo_hash, face_index, person_id, bbox_x, bbox_y, bbox_w, bbox_h, quality)
                 VALUES (?, ?, ?, 0.0, 0.0, 1.0, 1.0, 1.0)",
            )
            .bind(&photo.hash_sha256)
            .bind(index)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        }
        id
    }

    fn routes(
        pool: DbPool,
        cache: CacheManager,
    ) -> impl Filter<Extract = impl warp::Reply, Error = std::convert::Infallible> + Clone {
        build_people_routes(pool, cache).recover(handle_rejection)
    }

    #[tokio::test]
    async fn list_hides_short_and_hidden_people_by_default() {
        let pool = create_in_memory_pool().await.unwrap();
        seed_person(&pool, "Visible", false, 3).await;
        seed_person(&pool, "Short", false, 2).await;
        seed_person(&pool, "Hidden", true, 3).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));

        let response = warp::test::request()
            .path("/api/people")
            .reply(&routes(pool.clone(), cache.clone()))
            .await;
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["people"].as_array().unwrap().len(), 1);
        assert_eq!(body["people"][0]["name"], "Visible");

        let response = warp::test::request()
            .path("/api/people?include_hidden=true")
            .reply(&routes(pool, cache))
            .await;
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["people"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn patch_renames_and_hides() {
        let pool = create_in_memory_pool().await.unwrap();
        let id = seed_person(&pool, "Old", false, 3).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/people/{id}"))
            .json(&serde_json::json!({"name": " Anna ", "hidden": true}))
            .reply(&routes(pool.clone(), cache.clone()))
            .await;
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["person"]["name"], "Anna");
        assert_eq!(body["person"]["hidden"], true);
    }

    #[tokio::test]
    async fn patch_rejects_an_overlong_name() {
        let pool = create_in_memory_pool().await.unwrap();
        let id = seed_person(&pool, "Old", false, 3).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));

        let response = warp::test::request()
            .method("PATCH")
            .path(&format!("/api/people/{id}"))
            .json(&serde_json::json!({"name": "x".repeat(101)}))
            .reply(&routes(pool, cache))
            .await;
        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn missing_crop_is_404() {
        let pool = create_in_memory_pool().await.unwrap();
        let id = seed_person(&pool, "Visible", false, 3).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));
        let face_id: i64 = sqlx::query_scalar("SELECT id FROM faces WHERE person_id = ? LIMIT 1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();

        let response = warp::test::request()
            .path(&format!("/api/faces/{face_id}/crop"))
            .reply(&routes(pool, cache))
            .await;
        assert_eq!(response.status(), 404);
    }

    #[tokio::test]
    async fn merge_endpoint_moves_faces_and_exposes_the_undo_slot() {
        let pool = create_in_memory_pool().await.unwrap();
        let keeper = seed_person(&pool, "Anna", false, 3).await;
        let source = seed_person(&pool, "Oma", false, 2).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));

        let response = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({
                "target_id": keeper,
                "source_ids": [source],
                "name": "Anna",
            }))
            .reply(&routes(pool.clone(), cache.clone()))
            .await;
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["moved_faces"], 2);
        assert_eq!(body["removed_people"], 1);
        let undo_id = body["undo_id"].as_i64().unwrap();

        let response = warp::test::request()
            .path("/api/people?include_hidden=true")
            .reply(&routes(pool.clone(), cache))
            .await;
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["people"].as_array().unwrap().len(), 1);
        assert_eq!(body["people"][0]["id"], keeper);
        assert_eq!(body["people"][0]["face_count"], 5);
        assert_eq!(body["last_merge"]["undo_id"], undo_id);
        assert_eq!(body["last_merge"]["target_id"], keeper);
        assert_eq!(body["last_merge"]["target_name"], "Anna");
        assert_eq!(body["last_merge"]["source_count"], 1);
    }

    #[tokio::test]
    async fn merge_rejects_invalid_selections() {
        let pool = create_in_memory_pool().await.unwrap();
        let keeper = seed_person(&pool, "Anna", false, 3).await;
        let source = seed_person(&pool, "Oma", false, 3).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));
        let filter = routes(pool.clone(), cache);

        let single = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({"target_id": keeper, "source_ids": []}))
            .reply(&filter)
            .await;
        assert_eq!(single.status(), 400);

        let duplicate = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({"target_id": keeper, "source_ids": [source, source]}))
            .reply(&filter)
            .await;
        assert_eq!(duplicate.status(), 400);

        let keeper_as_source = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({"target_id": keeper, "source_ids": [keeper, source]}))
            .reply(&filter)
            .await;
        assert_eq!(keeper_as_source.status(), 400);

        let mut too_many: Vec<i64> = Vec::new();
        for _ in 0..=MAX_MERGE_PERSONS {
            let id: i64 = sqlx::query_scalar("INSERT INTO people DEFAULT VALUES RETURNING id")
                .fetch_one(&pool)
                .await
                .unwrap();
            too_many.push(id);
        }
        let oversized = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({"target_id": too_many[0], "source_ids": &too_many[1..]}))
            .reply(&filter)
            .await;
        assert_eq!(oversized.status(), 400);

        let overlong_name = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({
                "target_id": keeper,
                "source_ids": [source],
                "name": "x".repeat(101),
            }))
            .reply(&filter)
            .await;
        assert_eq!(overlong_name.status(), 400);

        let unknown = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({"target_id": keeper, "source_ids": [9999]}))
            .reply(&filter)
            .await;
        assert_eq!(unknown.status(), 404);
    }

    #[tokio::test]
    async fn undo_endpoint_reports_missing_and_stale_slots() {
        let pool = create_in_memory_pool().await.unwrap();
        let keeper = seed_person(&pool, "Anna", false, 3).await;
        let source = seed_person(&pool, "Oma", false, 2).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));
        let filter = routes(pool.clone(), cache);

        let missing = warp::test::request()
            .method("POST")
            .path("/api/people/merge/undo")
            .json(&serde_json::json!({"undo_id": 1}))
            .reply(&filter)
            .await;
        assert_eq!(missing.status(), 404);

        let merged = warp::test::request()
            .method("POST")
            .path("/api/people/merge")
            .json(&serde_json::json!({"target_id": keeper, "source_ids": [source]}))
            .reply(&filter)
            .await;
        let body: serde_json::Value = serde_json::from_slice(merged.body()).unwrap();
        let undo_id = body["undo_id"].as_i64().unwrap();

        let stale = warp::test::request()
            .method("POST")
            .path("/api/people/merge/undo")
            .json(&serde_json::json!({"undo_id": undo_id + 100}))
            .reply(&filter)
            .await;
        assert_eq!(stale.status(), 409);

        let undone = warp::test::request()
            .method("POST")
            .path("/api/people/merge/undo")
            .json(&serde_json::json!({"undo_id": undo_id}))
            .reply(&filter)
            .await;
        assert_eq!(undone.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(undone.body()).unwrap();
        assert_eq!(body["restored_faces"], 2);
        assert_eq!(body["total_faces"], 2);
        assert_eq!(body["restored_people"], 1);

        let consumed = warp::test::request()
            .method("POST")
            .path("/api/people/merge/undo")
            .json(&serde_json::json!({"undo_id": undo_id}))
            .reply(&filter)
            .await;
        assert_eq!(consumed.status(), 404);

        let list = warp::test::request()
            .path("/api/people?include_hidden=true")
            .reply(&filter)
            .await;
        let body: serde_json::Value = serde_json::from_slice(list.body()).unwrap();
        assert!(body["last_merge"].is_null());
    }

    #[tokio::test]
    async fn bulk_hide_endpoint_updates_known_ids_and_rejects_invalid_lists() {
        let pool = create_in_memory_pool().await.unwrap();
        let first = seed_person(&pool, "Anna", false, 3).await;
        let second = seed_person(&pool, "Oma", false, 3).await;
        let cache = CacheManager::new(std::env::temp_dir().join("face-api-test-cache"));
        let filter = routes(pool.clone(), cache);

        let response = warp::test::request()
            .method("POST")
            .path("/api/people/batch/hide")
            .json(&serde_json::json!({"ids": [first, second, 9999], "hidden": true}))
            .reply(&filter)
            .await;
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["updated"], 2);

        let list = warp::test::request()
            .path("/api/people")
            .reply(&filter)
            .await;
        let body: serde_json::Value = serde_json::from_slice(list.body()).unwrap();
        assert_eq!(body["people"].as_array().unwrap().len(), 0);

        let empty = warp::test::request()
            .method("POST")
            .path("/api/people/batch/hide")
            .json(&serde_json::json!({"ids": [], "hidden": true}))
            .reply(&filter)
            .await;
        assert_eq!(empty.status(), 400);

        let oversized = warp::test::request()
            .method("POST")
            .path("/api/people/batch/hide")
            .json(&serde_json::json!({"ids": vec![1; MAX_BULK_PEOPLE + 1], "hidden": true}))
            .reply(&filter)
            .await;
        assert_eq!(oversized.status(), 400);
    }
}
