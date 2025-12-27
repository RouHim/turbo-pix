use log::{info, warn};
use rusqlite::params;
use std::collections::HashSet;
use std::sync::Arc;

use crate::db::DbPool;
use crate::semantic_search::SemanticSearchEngine;

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
    semantic_search: &Arc<SemanticSearchEngine>,
) -> Result<usize, Box<dyn std::error::Error>> {
    info!("Starting housekeeping candidate identification scan...");
    
    // We will collect all candidates first to avoid holding a DB lock for too long
    // while querying semantic search (which might be fast, but good practice).
    // Structure: Photo Hash -> (Reason, Score)
    let mut candidates: Vec<(String, String, f32)> = Vec::new();
    let mut unique_paths: HashSet<String> = HashSet::new();

    // DEBUG: Check if we have any semantic vectors at all
    {
        let conn = db_pool.get()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM media_semantic_vectors",
            [],
            |row| row.get(0)
        ).unwrap_or(0);
        info!("DEBUG: media_semantic_vectors count: {}", count);
    }

    for &term in HOUSEKEEPING_TERMS {
        // Search for the term
        match semantic_search.search(term, MAX_RESULTS_PER_TERM, 0) {
            Ok(results) => {
                info!("Found {} results for housekeeping term '{}'", results.len(), term);
                for (path, score) in results {
                    // We need to map path to hash. We'll do this in bulk or per item?
                    // Let's store path for now and resolve to hash later.
                    candidates.push((path.clone(), term.to_string(), score));
                    unique_paths.insert(path);
                }
            }
            Err(e) => {
                warn!("Failed to search for housekeeping term '{}': {}", term, e);
            }
        }
    }

    // Now write to database
    let mut conn = db_pool.get()?;
    let tx = conn.transaction()?;

    // 1. Clear existing candidates
    // Always clear table to ensure a fresh list, even if no new candidates are found.
    tx.execute("DELETE FROM housekeeping_candidates", [])?;

    if candidates.is_empty() {
        tx.commit()?;
        info!("No housekeeping candidates found.");
        return Ok(0);
    }

    let mut inserted_count = 0;

    // Resolve paths to hashes and insert
    // We prepare a statement to look up hash by path
    {
        let mut stmt_get_hash = tx.prepare("SELECT hash_sha256 FROM photos WHERE file_path = ?")?;
        let mut stmt_insert = tx.prepare(
            "INSERT OR IGNORE INTO housekeeping_candidates (photo_hash, reason, score) VALUES (?, ?, ?)"
        )?;

        for (path, reason, score) in candidates {
            // Find hash for path
            let hash_result: Result<String, _> = stmt_get_hash.query_row(params![path], |row| row.get(0));

            match hash_result {
                Ok(hash) => {
                    stmt_insert.execute(params![hash, reason, score])?;
                    inserted_count += 1;
                }
                Err(_) => {
                    // Photo might have been deleted or path is stale in vector index?
                    // Just ignore.
                }
            }
        }
    }

    tx.commit()?;

    info!("Housekeeping scan completed. Identified {} candidates.", inserted_count);

    Ok(inserted_count)
}