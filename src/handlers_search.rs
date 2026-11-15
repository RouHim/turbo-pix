use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp::{reject, Filter, Rejection, Reply};

use crate::db::DbPool;
use crate::semantic_search::SemanticSearch;
use crate::warp_helpers::{with_db, with_semantic_search, DatabaseError};

/// Default page size for semantic search responses.
const DEFAULT_LIMIT: usize = 50;

/// Clamp bounds for client-supplied pagination: an unbounded limit would
/// stream the whole vector index back in one response.
const MIN_LIMIT: usize = 1;
const MAX_LIMIT: usize = 200;
const MAX_OFFSET: usize = 1_000_000;

#[derive(Debug, Deserialize)]
pub struct SemanticSearchQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_offset")]
    pub offset: usize,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
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
    semantic_search: Arc<dyn SemanticSearch>,
) -> Result<impl Reply, Rejection> {
    log::info!(
        "Semantic search query: '{}' (limit: {}, offset: {})",
        query.q,
        query.limit,
        query.offset
    );

    // Perform semantic search. Clamp the client-supplied pagination: an
    // unbounded limit would stream the whole vector index back in one
    // response.
    let limit = query.limit.clamp(MIN_LIMIT, MAX_LIMIT);
    let offset = query.offset.min(MAX_OFFSET);
    let results = semantic_search
        .search(&query.q, limit, offset)
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

    let mut query_builder = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(query_sql));
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
    semantic_search_engine: Arc<dyn SemanticSearch>,
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

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use candle_core::Tensor;
    use warp::http::StatusCode;

    use super::*;
    use crate::db::create_in_memory_pool;
    use crate::db::tests::create_test_photo;
    use crate::semantic_search::VideoSemanticMeta;
    use crate::warp_helpers::handle_rejection;

    /// A semantic-search stand-in that records the pagination the handler
    /// passed on and slices a fixed hit list with it, so an assertion can name
    /// the effective page size instead of only the status code. It never
    /// touches the CLIP model, so no test reaches the network.
    struct RecordingSemanticSearch {
        hits: Vec<(String, f32)>,
        calls: Mutex<Vec<(usize, usize)>>,
        failure: Option<String>,
    }

    impl RecordingSemanticSearch {
        fn new(hits: Vec<(String, f32)>) -> Self {
            Self {
                hits,
                calls: Mutex::new(Vec::new()),
                failure: None,
            }
        }

        /// Every search fails, like a server whose CLIP weights could not load.
        fn failing(message: &str) -> Self {
            Self {
                hits: Vec::new(),
                calls: Mutex::new(Vec::new()),
                failure: Some(message.to_string()),
            }
        }

        /// `(limit, offset)` per call, in request order.
        fn calls(&self) -> Vec<(usize, usize)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SemanticSearch for RecordingSemanticSearch {
        async fn search(
            &self,
            _query: &str,
            limit: usize,
            offset: usize,
        ) -> anyhow::Result<Vec<(String, f32)>> {
            self.calls.lock().unwrap().push((limit, offset));
            if let Some(message) = &self.failure {
                return Err(anyhow::anyhow!("{message}"));
            }
            Ok(self.hits.iter().skip(offset).take(limit).cloned().collect())
        }

        async fn encode_image_vector(&self, _image_path: &str) -> anyhow::Result<(String, Tensor)> {
            Err(anyhow::anyhow!("encoding is not available in these tests"))
        }

        async fn encode_video_vector(
            &self,
            _video_path: &str,
            _frame_count: Option<usize>,
        ) -> anyhow::Result<(String, Tensor, VideoSemanticMeta)> {
            Err(anyhow::anyhow!("encoding is not available in these tests"))
        }
    }

    fn build_test_routes(
        db_pool: DbPool,
        engine: Arc<RecordingSemanticSearch>,
    ) -> impl Filter<Extract = impl warp::Reply, Error = Infallible> + Clone {
        build_search_routes(db_pool, engine).recover(handle_rejection)
    }

    /// Seeds one photo per name and returns their stored `file_path`s, which is
    /// what the search engine reports back as hits.
    async fn seed_photos(db_pool: &DbPool, names: &[&str]) -> Vec<String> {
        let mut paths = Vec::new();
        for name in names {
            let photo = create_test_photo(format!("{name}.jpg"), (*name).to_string());
            paths.push(photo.file_path.clone());
            photo.create(db_pool).await.unwrap();
        }
        paths
    }

    /// Hits in seed order, best score first.
    fn hits_for(paths: &[String]) -> Vec<(String, f32)> {
        paths
            .iter()
            .enumerate()
            .map(|(index, path)| (path.clone(), 1.0 - index as f32 * 0.1))
            .collect()
    }

    // GIVEN three indexed photos and a search that answers with all of them
    // WHEN the same query is asked for with limit=0 and with limit=100000
    // THEN each request is clamped to the handler's bounds (1 and MAX_LIMIT),
    //      and the effective page size follows the clamp, not the request
    #[tokio::test]
    async fn limit_is_clamped_to_the_handler_bounds() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let paths = seed_photos(&db_pool, &["a", "b", "c"]).await;
        let engine = Arc::new(RecordingSemanticSearch::new(hits_for(&paths)));
        let routes = build_test_routes(db_pool, engine.clone());

        let zero = warp::test::request()
            .path("/api/search/semantic?q=cat&limit=0")
            .reply(&routes)
            .await;
        assert_eq!(zero.status(), StatusCode::OK);
        let zero_body: serde_json::Value = serde_json::from_slice(zero.body()).unwrap();
        assert_eq!(zero_body["total"], 1);
        assert_eq!(zero_body["results"].as_array().unwrap().len(), 1);
        assert_eq!(zero_body["results"][0]["path"], "./test/a.jpg");

        let huge = warp::test::request()
            .path("/api/search/semantic?q=cat&limit=100000")
            .reply(&routes)
            .await;
        assert_eq!(huge.status(), StatusCode::OK);
        let huge_body: serde_json::Value = serde_json::from_slice(huge.body()).unwrap();
        assert_eq!(huge_body["query"], "cat");
        assert_eq!(huge_body["total"], 3);
        assert_eq!(huge_body["results"].as_array().unwrap().len(), 3);

        assert_eq!(engine.calls(), vec![(MIN_LIMIT, 0), (MAX_LIMIT, 0)]);
    }

    // GIVEN two indexed photos
    // WHEN the offset is past the end of the result set (beyond MAX_OFFSET)
    // THEN the reply is an empty page with 200 — not a 500 — and the engine saw
    //      the default page size and the clamped offset
    #[tokio::test]
    async fn offset_beyond_the_result_set_is_an_empty_page() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let paths = seed_photos(&db_pool, &["a", "b"]).await;
        let engine = Arc::new(RecordingSemanticSearch::new(hits_for(&paths)));
        let routes = build_test_routes(db_pool, engine.clone());

        let response = warp::test::request()
            .path("/api/search/semantic?q=cat&offset=5000000")
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::OK);

        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["query"], "cat");
        assert_eq!(body["total"], 0);
        assert_eq!(body["results"].as_array().unwrap().len(), 0);
        assert_eq!(engine.calls(), vec![(DEFAULT_LIMIT, MAX_OFFSET)]);
    }

    // GIVEN a search engine that cannot answer (the CLIP weights failed to load)
    // WHEN a semantic search is requested
    // THEN it maps to 500 with the sanitized "Database error" body and code
    #[tokio::test]
    async fn a_failing_embedding_provider_is_a_database_error() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let engine = Arc::new(RecordingSemanticSearch::failing("model unavailable"));
        let routes = build_test_routes(db_pool, engine.clone());

        let response = warp::test::request()
            .path("/api/search/semantic?q=cat")
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error"], "Database error");
        assert_eq!(body["code"], 500);
        assert!(
            body.get("error_code").is_none(),
            "generic errors carry no machine-readable error_code: {body}"
        );
        assert_eq!(engine.calls(), vec![(DEFAULT_LIMIT, 0)]);
    }

    // GIVEN a search that reports a hit but a database that cannot resolve it
    // WHEN the handler looks the hit's hash up
    // THEN the query failure maps to 500 with the sanitized body, not a partial
    //      or empty result
    #[tokio::test]
    async fn a_failing_hash_lookup_is_a_database_error() {
        let db_pool = create_in_memory_pool().await.unwrap();
        let engine = Arc::new(RecordingSemanticSearch::new(vec![(
            "./test/a.jpg".to_string(),
            0.9,
        )]));
        let routes = build_test_routes(db_pool.clone(), engine);
        db_pool.close().await;

        let response = warp::test::request()
            .path("/api/search/semantic?q=cat")
            .reply(&routes)
            .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["error"], "Database error");
        assert_eq!(body["code"], 500);
        assert!(body.get("results").is_none());
    }
}
