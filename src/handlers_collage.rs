use log::info;
use std::path::PathBuf;
use std::sync::Arc;
use warp::{reject, Filter, Rejection, Reply};

use crate::collage_generator::{self, Collage};
use crate::db::DbPool;
use crate::handlers_photo::{BatchFailure, BatchResult};
use crate::media_facts::MediaFactsIndex;
use crate::semantic_search::SemanticSearch;
use crate::warp_helpers::{with_db, with_facts, DatabaseError};

#[derive(Debug, serde::Deserialize)]
pub struct BatchCollageIdsRequest {
    pub ids: Vec<i64>,
}

/// Shared batch-size validation for collage batch endpoints (same contract
/// as the photo batch endpoints).
fn validate_collage_ids(ids: &[i64]) -> Result<(), Rejection> {
    if ids.is_empty() {
        return Err(reject::custom(crate::warp_helpers::ValidationError {
            message: "ids must not be empty".to_string(),
        }));
    }
    if ids.len() > 1000 {
        return Err(reject::custom(crate::warp_helpers::ValidationError {
            message: "too many ids (max 1000)".to_string(),
        }));
    }
    Ok(())
}

/// Batch-accept pending collages. `accept_collage` is idempotent for settled
/// collages (returns the existing destination path), so a double-submit can
/// never corrupt state — it is merely counted as applied again.
pub async fn batch_accept_collages(
    req: BatchCollageIdsRequest,
    db_pool: DbPool,
    data_path: PathBuf,
    semantic_search: Arc<dyn SemanticSearch>,
    facts: Arc<MediaFactsIndex>,
) -> Result<impl Reply, Rejection> {
    validate_collage_ids(&req.ids)?;

    let mut result = BatchResult {
        applied: Vec::new(),
        failed: Vec::new(),
    };

    for id in &req.ids {
        match collage_generator::accept_collage(
            &db_pool,
            *id,
            &data_path,
            semantic_search.clone(),
            &facts,
        )
        .await
        {
            Ok(_) => result.applied.push(id.to_string()),
            Err(e) => {
                log::error!("Failed to accept collage {}: {}", id, e);
                result.failed.push(BatchFailure {
                    id: id.to_string(),
                    error: e.to_string(),
                });
            }
        }
    }

    Ok(warp::reply::json(&result))
}

/// Batch-reject pending collages.
pub async fn batch_reject_collages(
    req: BatchCollageIdsRequest,
    db_pool: DbPool,
) -> Result<impl Reply, Rejection> {
    validate_collage_ids(&req.ids)?;

    let mut result = BatchResult {
        applied: Vec::new(),
        failed: Vec::new(),
    };

    for id in &req.ids {
        match collage_generator::reject_collage(&db_pool, *id).await {
            Ok(_) => result.applied.push(id.to_string()),
            Err(e) => {
                log::error!("Failed to reject collage {}: {}", id, e);
                result.failed.push(BatchFailure {
                    id: id.to_string(),
                    error: e.to_string(),
                });
            }
        }
    }

    Ok(warp::reply::json(&result))
}

/// List all pending collages
pub async fn list_pending_collages(db_pool: DbPool) -> Result<impl Reply, Rejection> {
    match Collage::list_pending_cleaned(&db_pool).await {
        Ok(collages) => Ok(warp::reply::json(&collages)),
        Err(e) => {
            log::error!("Failed to list pending collages: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }))
        }
    }
}

/// Accept a collage (move to photos directory and trigger indexing)
pub async fn accept_collage(
    id: i64,
    db_pool: DbPool,
    data_path: PathBuf,
    semantic_search: Arc<dyn SemanticSearch>,
    facts: Arc<MediaFactsIndex>,
) -> Result<impl Reply, Rejection> {
    info!("Accepting collage {}", id);

    // Move collage to photos directory and index immediately
    let accepted_path =
        match collage_generator::accept_collage(&db_pool, id, &data_path, semantic_search, &facts)
            .await
        {
            Ok(path) => path,
            Err(e) => {
                log::error!("Failed to accept collage: {}", e);
                return Err(reject::custom(DatabaseError {
                    message: format!("Failed to accept collage: {}", e),
                }));
            }
        };

    info!(
        "Collage accepted, indexed, and moved to {:?}",
        accepted_path
    );

    Ok(warp::reply::json(&serde_json::json!({
        "success": true,
        "message": "Collage accepted and added to 'All Photos'.",
        "path": accepted_path.to_string_lossy()
    })))
}

/// Reject a collage (delete files and mark as rejected)
pub async fn reject_collage(id: i64, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    info!("Rejecting collage {}", id);

    match collage_generator::reject_collage(&db_pool, id).await {
        Ok(_) => Ok(warp::reply::json(&serde_json::json!({
            "success": true,
            "message": "Collage rejected and deleted"
        }))),
        Err(e) => {
            log::error!("Failed to reject collage: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to reject collage: {}", e),
            }))
        }
    }
}

/// Get collage image file
pub async fn get_collage_image(id: i64, db_pool: DbPool) -> Result<impl Reply, Rejection> {
    // Find the collage
    let collage = match Collage::get_by_id(&db_pool, id).await {
        Ok(Some(collage)) => collage,
        Ok(None) => {
            log::error!("Collage not found: {}", id);
            return Err(reject::not_found());
        }
        Err(e) => {
            log::error!("Failed to find collage: {}", e);
            return Err(reject::custom(DatabaseError {
                message: format!("Database error: {}", e),
            }));
        }
    };

    let file_path = std::path::Path::new(&collage.file_path);

    // Check if file exists
    if !file_path.exists() {
        log::error!("Collage file not found: {:?}", file_path);
        return Err(reject::not_found());
    }

    // Stream the file instead of buffering it (same pattern as the photo
    // file route): unauthenticated clients could otherwise force unbounded
    // per-request allocations with many concurrent requests.
    let file = match tokio::fs::File::open(file_path).await {
        Ok(file) => file,
        Err(_) => return Err(reject::not_found()),
    };
    let file_size = file.metadata().await.map(|m| m.len()).unwrap_or(0);

    // Return image with appropriate headers
    let reply = warp::reply::stream(tokio_util::io::ReaderStream::new(file));
    let reply = warp::reply::with_header(reply, "content-type", "image/jpeg");
    let reply = warp::reply::with_header(reply, "content-length", file_size.to_string());
    let reply = warp::reply::with_header(reply, "cache-control", "public, max-age=31536000");

    Ok(reply)
}

/// Manually trigger collage generation (for testing)
pub async fn generate_collages_manual(
    db_pool: DbPool,
    facts: Arc<MediaFactsIndex>,
    data_path: PathBuf,
    locale: String,
) -> Result<impl Reply, Rejection> {
    info!("Manual collage generation triggered");

    match collage_generator::generate_collages(&db_pool, &facts, &data_path, &locale).await {
        Ok(count) => {
            info!(
                "Manual collage generation completed: {} collages created",
                count
            );
            Ok(warp::reply::json(&serde_json::json!({
                "success": true,
                "count": count,
                "message": format!("{} collage(s) generated", count)
            })))
        }
        Err(e) => {
            log::error!("Failed to generate collages: {}", e);
            Err(reject::custom(DatabaseError {
                message: format!("Failed to generate collages: {}", e),
            }))
        }
    }
}

/// Build collage routes
pub fn build_collage_routes(
    db_pool: DbPool,
    media_facts: Arc<MediaFactsIndex>,
    data_path: PathBuf,
    locale: String,
    semantic_search: Arc<dyn SemanticSearch>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
    let list_pending = warp::path!("api" / "collages" / "pending")
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(list_pending_collages);

    let get_image = warp::path!("api" / "collages" / i64 / "image")
        .and(warp::get())
        .and(with_db(db_pool.clone()))
        .and_then(get_collage_image);

    let generate = {
        let data_path = data_path.clone();
        let locale = locale.clone();
        let media_facts = media_facts.clone();
        warp::path!("api" / "collages" / "generate")
            .and(warp::post())
            .and(with_db(db_pool.clone()))
            .and(with_facts(media_facts))
            .map(move |db_pool, facts| (db_pool, facts, data_path.clone(), locale.clone()))
            .untuple_one()
            .and_then(generate_collages_manual)
    };

    let accept = {
        let data_path = data_path.clone();
        let semantic_search = semantic_search.clone();
        let media_facts = media_facts.clone();
        warp::path!("api" / "collages" / i64 / "accept")
            .and(warp::post())
            .and(with_db(db_pool.clone()))
            .and(with_facts(media_facts))
            .map(move |id, db_pool, facts| {
                (
                    id,
                    db_pool,
                    data_path.clone(),
                    semantic_search.clone(),
                    facts,
                )
            })
            .untuple_one()
            .and_then(accept_collage)
    };

    let reject = warp::path!("api" / "collages" / i64 / "reject")
        .and(warp::delete())
        .and(with_db(db_pool.clone()))
        .and_then(reject_collage);

    // Literal batch routes registered BEFORE the `{id}` param routes so
    // `/api/collages/batch-accept` is never parsed as `accept(0)`.
    let batch_accept = {
        let data_path = data_path.clone();
        let semantic_search = semantic_search.clone();
        let media_facts = media_facts.clone();
        warp::path!("api" / "collages" / "batch-accept")
            .and(warp::post())
            .and(warp::body::content_length_limit(1024 * 1024))
            .and(warp::body::json::<BatchCollageIdsRequest>())
            .and(with_db(db_pool.clone()))
            .and(with_facts(media_facts))
            .map(move |req, db, facts| (req, db, data_path.clone(), semantic_search.clone(), facts))
            .untuple_one()
            .and_then(batch_accept_collages)
    };

    let batch_reject = warp::path!("api" / "collages" / "batch-reject")
        .and(warp::post())
        .and(warp::body::content_length_limit(1024 * 1024))
        .and(warp::body::json::<BatchCollageIdsRequest>())
        .and(with_db(db_pool.clone()))
        .and_then(batch_reject_collages);

    list_pending
        .or(get_image)
        .or(generate)
        .or(batch_accept)
        .or(batch_reject)
        .or(accept)
        .or(reject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::create_in_memory_pool;
    use crate::semantic_search::NoopSemanticSearch;
    use crate::warp_helpers::handle_rejection;
    use std::convert::Infallible;
    use std::fs;
    use tempfile::TempDir;

    fn build_test_routes(
        db_pool: DbPool,
        data_path: PathBuf,
        semantic_search: Arc<dyn SemanticSearch>,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        build_test_routes_with_facts(
            db_pool,
            data_path,
            semantic_search,
            Arc::new(MediaFactsIndex::new()),
        )
    }

    /// Same route set, serving capture facts from `facts` — the tests that
    /// assert an accepted collage's date must inspect the index it publishes
    /// into.
    fn build_test_routes_with_facts(
        db_pool: DbPool,
        data_path: PathBuf,
        semantic_search: Arc<dyn SemanticSearch>,
        facts: Arc<MediaFactsIndex>,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        build_collage_routes(db_pool, facts, data_path, "en".to_string(), semantic_search)
            .recover(handle_rejection)
    }

    /// Seed a pending collage backed by a real staging file; returns its id.
    async fn seed_pending_collage(pool: &DbPool, staging_dir: &std::path::Path, n: usize) -> i64 {
        let file_path = staging_dir.join(format!("collage_batch_{}.jpg", n));
        fs::write(&file_path, b"collage-test-bytes").expect("Failed to write staging file");
        Collage::insert(
            pool,
            "2026-08-01",
            &file_path.to_string_lossy(),
            None,
            3,
            &[format!("batch-hash-{}", n)],
            &format!("batch-sig-{}", n),
        )
        .await
        .expect("Failed to insert collage")
    }

    /// GIVEN a pending collage whose file carries a capture date
    /// WHEN it is accepted through the API
    /// THEN the accepted collage is queryable through the index with the
    ///      extracted date right away — no rescan — and a month filter for
    ///      that month matches it.
    #[tokio::test]
    async fn test_accept_collage_publishes_the_extracted_date_to_facts() {
        use crate::db::{Photo, SearchQuery};
        use chrono::{TimeZone, Utc};

        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let staging_dir = temp_dir.path().join("collages").join("staging");
        fs::create_dir_all(&staging_dir).unwrap();

        // A real, EXIF-carrying file: the accept path extracts the date from
        // the file itself (the DB row stores none).
        let file_path = staging_dir.join("collage_2026-08-01_1.jpg");
        fs::copy("test-data/IMG_9377.jpg", &file_path).unwrap();
        let captured = Utc.with_ymd_and_hms(2015, 6, 20, 10, 0, 0).unwrap();
        crate::metadata_writer::update_metadata(&file_path, Some(captured), None, None)
            .expect("failed to write the capture date into the file");

        let hashes = vec!["hash1".to_string()];
        let collage_id = Collage::insert(
            &db_pool,
            "2026-08-01",
            &file_path.to_string_lossy(),
            None,
            3,
            &hashes,
            "accept-publishes-date-sig",
        )
        .await
        .expect("Failed to insert collage");

        let facts = Arc::new(MediaFactsIndex::new());
        let routes = build_test_routes_with_facts(
            db_pool.clone(),
            temp_dir.path().to_path_buf(),
            Arc::new(NoopSemanticSearch),
            facts.clone(),
        );

        // WHEN: the collage is accepted
        let response = warp::test::request()
            .method("POST")
            .path(&format!("/api/collages/{}/accept", collage_id))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        let dest = body["path"]
            .as_str()
            .expect("accept returns the destination");
        assert_eq!(
            std::path::Path::new(dest).file_name().unwrap(),
            "collage_2026-08-01_1.jpg"
        );

        // THEN: the index carries the date the file holds...
        let published = facts
            .get(dest)
            .expect("the accepted collage's facts must be published");
        assert_eq!(
            published.taken_at,
            Some(captured),
            "the extracted date must be published with the row write"
        );

        // ...and a month filter for that date matches the new photo.
        let (photos, total) = Photo::search_photos(
            &db_pool,
            &facts,
            &SearchQuery {
                q: None,
                year: Some(2015),
                month: Some(6),
                to_year: None,
                to_month: None,
            },
            50,
            0,
            None,
            None,
        )
        .await
        .expect("search must succeed");
        assert_eq!(total, 1, "the accepted collage must match its own month");
        assert_eq!(photos[0].file_path, dest);
        assert_eq!(photos[0].taken_at, Some(captured));

        // The undated-path control: a different month must not match it.
        let (other, other_total) = Photo::search_photos(
            &db_pool,
            &facts,
            &SearchQuery {
                q: None,
                year: Some(2020),
                month: Some(1),
                to_year: None,
                to_month: None,
            },
            50,
            0,
            None,
            None,
        )
        .await
        .expect("search must succeed");
        assert_eq!(other_total, 0);
        assert!(other.is_empty());
    }

    #[tokio::test]
    async fn test_batch_accept_collages_applies_and_settles() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let staging_dir = temp_dir.path().join("collages").join("staging");
        fs::create_dir_all(&staging_dir).unwrap();
        let id_a = seed_pending_collage(&db_pool, &staging_dir, 1).await;
        let id_b = seed_pending_collage(&db_pool, &staging_dir, 2).await;
        let routes = build_test_routes(
            db_pool.clone(),
            temp_dir.path().to_path_buf(),
            Arc::new(NoopSemanticSearch),
        );

        let response = warp::test::request()
            .method("POST")
            .path("/api/collages/batch-accept")
            .json(&serde_json::json!({"ids": [id_a, id_b]}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let result: BatchResult = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(result.applied.len(), 2);
        assert!(result.applied.contains(&id_a.to_string()));
        assert!(result.applied.contains(&id_b.to_string()));
        assert!(result.failed.is_empty());

        let collage_a = Collage::get_by_id(&db_pool, id_a).await.unwrap().unwrap();
        assert!(collage_a.accepted_at.is_some(), "collage must be settled");
        let collage_b = Collage::get_by_id(&db_pool, id_b).await.unwrap().unwrap();
        assert!(collage_b.accepted_at.is_some(), "collage must be settled");
    }

    #[tokio::test]
    async fn test_batch_reject_collages_applies_and_settles() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let staging_dir = temp_dir.path().join("collages").join("staging");
        fs::create_dir_all(&staging_dir).unwrap();
        let id_a = seed_pending_collage(&db_pool, &staging_dir, 1).await;
        let id_b = seed_pending_collage(&db_pool, &staging_dir, 2).await;
        let routes = build_test_routes(
            db_pool.clone(),
            temp_dir.path().to_path_buf(),
            Arc::new(NoopSemanticSearch),
        );

        let response = warp::test::request()
            .method("POST")
            .path("/api/collages/batch-reject")
            .json(&serde_json::json!({"ids": [id_a, id_b]}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 200);
        let result: BatchResult = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(result.applied.len(), 2);
        assert!(result.failed.is_empty());

        let collage_a = Collage::get_by_id(&db_pool, id_a).await.unwrap().unwrap();
        assert!(collage_a.rejected_at.is_some(), "collage must be settled");
        let collage_b = Collage::get_by_id(&db_pool, id_b).await.unwrap().unwrap();
        assert!(collage_b.rejected_at.is_some(), "collage must be settled");
    }

    #[tokio::test]
    async fn test_batch_collage_ids_rejects_empty() {
        let db_pool = create_in_memory_pool()
            .await
            .expect("Failed to create test database");
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let routes = build_test_routes(
            db_pool,
            temp_dir.path().to_path_buf(),
            Arc::new(NoopSemanticSearch),
        );

        let response = warp::test::request()
            .method("POST")
            .path("/api/collages/batch-accept")
            .json(&serde_json::json!({"ids": []}))
            .reply(&routes)
            .await;

        assert_eq!(response.status(), 400);
    }
}
