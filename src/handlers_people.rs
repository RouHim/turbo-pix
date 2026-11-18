//! People API: the clustered person cards and their cover crops.

use log::error;
use serde::{Deserialize, Serialize};
use warp::{reject, Filter, Rejection, Reply};

use crate::cache_manager::CacheManager;
use crate::db::DbPool;
use crate::face_recognition::store::{self, Person};
use crate::warp_helpers::{not_found_reply, with_cache, with_db, DatabaseError, ValidationError};

const MAX_NAME_CHARS: usize = 100;

#[derive(Debug, Deserialize)]
pub struct PeopleQuery {
    #[serde(default)]
    pub include_hidden: bool,
}

#[derive(Debug, Serialize)]
pub struct PeopleResponse {
    pub people: Vec<Person>,
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
    Ok(warp::reply::json(&PeopleResponse { people }))
}

pub async fn update_person(
    id: i64,
    update: PersonUpdate,
    db_pool: DbPool,
) -> Result<warp::reply::Response, Rejection> {
    let name = update.name.map(|name| {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    if name
        .as_ref()
        .and_then(|value| value.as_ref())
        .is_some_and(|value| value.chars().count() > MAX_NAME_CHARS)
    {
        return Err(reject::custom(ValidationError {
            message: format!("Person name must be at most {MAX_NAME_CHARS} characters"),
        }));
    }
    if name.is_none() && update.hidden.is_none() {
        return Err(reject::custom(ValidationError {
            message: "No person fields supplied".to_string(),
        }));
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
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({"error": "Database error"})),
                warp::http::StatusCode::INTERNAL_SERVER_ERROR,
            )
            .into_response());
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

    list.or(update).or(crop)
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
}
