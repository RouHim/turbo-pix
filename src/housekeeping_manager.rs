use log::{info, warn};
use std::sync::Arc;

use crate::db::DbPool;
use crate::semantic_search::SemanticSearch;

const HOUSEKEEPING_TERMS: &[&str] = &[
    "screenshot",
    "blurry image",
    "scanned document",
    "receipt",
    "invoice",
    "meme",
    "whiteboard",
    "qr code",
    "text message screenshot",
    "low quality image",
    "out of focus",
];

/// limit per term to avoid flooding
const MAX_RESULTS_PER_TERM: usize = 100;

pub async fn run_housekeeping_scan(
    db_pool: &DbPool,
    semantic_search: &Arc<dyn SemanticSearch>,
) -> Result<usize, Box<dyn std::error::Error>> {
    info!("Starting housekeeping candidate identification scan...");

    // We will collect all candidates first to avoid holding a DB lock for too long
    // while querying semantic search (which might be fast, but good practice).
    // Structure: Photo Hash -> (Reason, Score)
    let mut candidates: Vec<(String, String, f32)> = Vec::new();

    for &term in HOUSEKEEPING_TERMS {
        // Search for the term
        match semantic_search.search(term, MAX_RESULTS_PER_TERM, 0).await {
            Ok(results) => {
                info!(
                    "Found {} results for housekeeping term '{}'",
                    results.len(),
                    term
                );
                for (path, score) in results {
                    // Store paths for now; resolve to hashes below.
                    candidates.push((path.clone(), term.to_string(), score));
                }
            }
            Err(e) => {
                warn!("Failed to search for housekeeping term '{}': {}", term, e);
            }
        }
    }

    // Now write to database
    let mut tx = db_pool.begin().await?;

    // 1. Clear existing candidates
    // Always clear table to ensure a fresh list, even if no new candidates are found.
    sqlx::query("DELETE FROM housekeeping_candidates")
        .execute(&mut *tx)
        .await?;

    if candidates.is_empty() {
        tx.commit().await?;
        info!("No housekeeping candidates found.");
        return Ok(0);
    }

    let mut inserted_count = 0;

    // Resolve paths to hashes and insert
    for (path, reason, score) in candidates {
        // Find hash for path
        let hash_result: Result<String, sqlx::Error> =
            sqlx::query_scalar("SELECT hash_sha256 FROM photos WHERE file_path = ?")
                .bind(&path)
                .fetch_one(&mut *tx)
                .await;

        match hash_result {
            Ok(hash) => {
                sqlx::query(
                    "INSERT OR IGNORE INTO housekeeping_candidates (photo_hash, reason, score) VALUES (?, ?, ?)"
                )
                .bind(&hash)
                .bind(&reason)
                .bind(score)
                .execute(&mut *tx)
                .await?;
                inserted_count += 1;
            }
            Err(_) => {
                // Photo might have been deleted or path is stale in vector index?
                // Just ignore.
            }
        }
    }

    tx.commit().await?;

    info!(
        "Housekeeping scan completed. Identified {} candidates.",
        inserted_count
    );

    Ok(inserted_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_pool::create_in_memory_pool;
    use anyhow::anyhow;
    use async_trait::async_trait;
    use candle_core::Tensor;
    use std::collections::HashMap;

    /// Mock semantic search returning per-term (file_path, score) results.
    struct MockSemanticSearch {
        results: HashMap<String, Vec<(String, f32)>>,
        fail: bool,
    }

    #[async_trait]
    impl SemanticSearch for MockSemanticSearch {
        async fn search(
            &self,
            query: &str,
            _limit: usize,
            _offset: usize,
        ) -> anyhow::Result<Vec<(String, f32)>> {
            if self.fail {
                return Err(anyhow!("mock search failure"));
            }
            Ok(self.results.get(query).cloned().unwrap_or_default())
        }

        async fn encode_image_vector(&self, _image_path: &str) -> anyhow::Result<(String, Tensor)> {
            Err(anyhow!("not used by housekeeping scan"))
        }

        async fn encode_video_vector(
            &self,
            _video_path: &str,
            _frame_count: Option<usize>,
        ) -> anyhow::Result<(String, Tensor, crate::semantic_search::VideoSemanticMeta)> {
            Err(anyhow!("not used by housekeeping scan"))
        }
    }

    fn hash_for(n: u32) -> String {
        format!("{:0>64}", n)
    }

    async fn insert_photo(pool: &DbPool, hash: &str, path: &str) {
        sqlx::query(
            "INSERT INTO photos (hash_sha256, file_path, filename, file_size, file_modified) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(hash)
        .bind(path)
        .bind("photo.jpg")
        .bind(1024_i64)
        .bind("2024-01-01T00:00:00Z")
        .execute(pool)
        .await
        .unwrap();
    }

    async fn candidate_rows(pool: &DbPool) -> Vec<(String, String, f32)> {
        sqlx::query_as::<_, (String, String, f32)>(
            "SELECT photo_hash, reason, score FROM housekeeping_candidates ORDER BY photo_hash",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn inserts_candidates_resolving_paths_to_hashes() {
        // GIVEN two photos and a mock returning both for 'screenshot' and a
        // duplicate path for 'receipt' (dedupe is per-hash via INSERT OR IGNORE)
        let pool = create_in_memory_pool().await.unwrap();
        insert_photo(&pool, &hash_for(1), "/photos/one.jpg").await;
        insert_photo(&pool, &hash_for(2), "/photos/two.jpg").await;
        let search: Arc<dyn SemanticSearch> = Arc::new(MockSemanticSearch {
            results: HashMap::from([
                (
                    "screenshot".to_string(),
                    vec![
                        ("/photos/one.jpg".to_string(), 0.9),
                        ("/photos/two.jpg".to_string(), 0.8),
                    ],
                ),
                (
                    "receipt".to_string(),
                    vec![("/photos/one.jpg".to_string(), 0.7)],
                ),
            ]),
            fail: false,
        });

        // WHEN the scan runs
        let count = run_housekeeping_scan(&pool, &search).await.unwrap();

        // THEN every resolved path counts (including the ignored duplicate) and
        // the first matching reason/score wins per hash
        assert_eq!(count, 3);
        let rows = candidate_rows(&pool).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (hash_for(1), "screenshot".to_string(), 0.9));
        assert_eq!(rows[1], (hash_for(2), "screenshot".to_string(), 0.8));
    }

    #[tokio::test]
    async fn ignores_paths_not_in_photos_table() {
        // GIVEN one photo; the mock also returns a path with no photo row
        let pool = create_in_memory_pool().await.unwrap();
        insert_photo(&pool, &hash_for(1), "/photos/one.jpg").await;
        let search: Arc<dyn SemanticSearch> = Arc::new(MockSemanticSearch {
            results: HashMap::from([(
                "screenshot".to_string(),
                vec![
                    ("/photos/one.jpg".to_string(), 0.9),
                    ("/photos/missing.jpg".to_string(), 0.5),
                ],
            )]),
            fail: false,
        });

        let count = run_housekeeping_scan(&pool, &search).await.unwrap();

        assert_eq!(count, 1);
        let rows = candidate_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], (hash_for(1), "screenshot".to_string(), 0.9));
    }

    #[tokio::test]
    async fn clears_existing_candidates_when_no_results() {
        // GIVEN a pre-existing candidate and a mock returning nothing
        let pool = create_in_memory_pool().await.unwrap();
        insert_photo(&pool, &hash_for(1), "/photos/one.jpg").await;
        sqlx::query(
            "INSERT INTO housekeeping_candidates (photo_hash, reason, score) VALUES (?, ?, ?)",
        )
        .bind(hash_for(1))
        .bind("stale")
        .bind(0.5_f32)
        .execute(&pool)
        .await
        .unwrap();
        let search: Arc<dyn SemanticSearch> = Arc::new(MockSemanticSearch {
            results: HashMap::new(),
            fail: false,
        });

        let count = run_housekeeping_scan(&pool, &search).await.unwrap();

        assert_eq!(count, 0);
        assert!(candidate_rows(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn search_failure_is_tolerated() {
        // GIVEN a mock whose searches all fail
        let pool = create_in_memory_pool().await.unwrap();
        insert_photo(&pool, &hash_for(1), "/photos/one.jpg").await;
        let search: Arc<dyn SemanticSearch> = Arc::new(MockSemanticSearch {
            results: HashMap::new(),
            fail: true,
        });

        let count = run_housekeeping_scan(&pool, &search).await.unwrap();

        assert_eq!(count, 0);
        assert!(candidate_rows(&pool).await.is_empty());
    }
}
