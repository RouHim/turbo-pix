use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use turbo_pix::db::create_db_pool;
use turbo_pix::semantic_search::SemanticSearchEngine;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // Setup
    let db_path = "benchmark.db";
    let _ = std::fs::remove_file(db_path); // Clean start
    let pool = create_db_pool(db_path)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Initialize engine (loads models)
    println!("Loading models...");
    let start_load = Instant::now();
    let engine = Arc::new(SemanticSearchEngine::new(pool.clone(), "./data").await?);
    println!("Models loaded in {:.2?}", start_load.elapsed());

    let test_data = Path::new("test-data");
    let videos: Vec<_> = std::fs::read_dir(test_data)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase() == "mp4")
                .unwrap_or(false)
        })
        .collect();

    if videos.is_empty() {
        println!("No MP4 videos found in test-data/");
        return Ok(());
    }

    let frame_counts = [1, 3, 5, 10, 20];

    for video_path in videos {
        println!(
            "\nBenchmarking video: {:?}",
            video_path.file_name().unwrap()
        );
        let video_path_str = video_path.to_string_lossy().to_string();

        let mut embeddings = Vec::new();

        println!(
            "{:<10} | {:<15} | {:<15}",
            "Frames", "Time (ms)", "FPS equivalent"
        );
        println!("{:-<10}-+-{:-<15}-+-{:-<15}", "", "", "");

        for &count in &frame_counts {
            // Clear cache for this video to force re-computation
            let id: Option<i64> =
                sqlx::query_scalar("SELECT id FROM semantic_vector_path_mapping WHERE path = ?")
                    .bind(&video_path_str)
                    .fetch_optional(&pool)
                    .await?;

            if let Some(id) = id {
                sqlx::query("DELETE FROM media_semantic_vectors WHERE rowid = ?")
                    .bind(id)
                    .execute(&pool)
                    .await?;
                sqlx::query("DELETE FROM semantic_vector_path_mapping WHERE id = ?")
                    .bind(id)
                    .execute(&pool)
                    .await?;
            }
            sqlx::query("DELETE FROM video_semantic_metadata WHERE path = ?")
                .bind(&video_path_str)
                .execute(&pool)
                .await?;

            let start = Instant::now();
            engine
                .compute_video_semantic_vector(&video_path_str, Some(count))
                .await
                .with_context(|| format!("Failed to compute video for count {}", count))?;
            let duration = start.elapsed();

            println!("{:<10} | {:<15.2} |", count, duration.as_millis());

            // Retrieve the computed embedding for comparison
            let embedding_bytes: Vec<u8> = sqlx::query_scalar(
                "SELECT msv.semantic_vector
                 FROM media_semantic_vectors msv
                 JOIN semantic_vector_path_mapping ic ON msv.rowid = ic.id
                 WHERE ic.path = ?",
            )
            .bind(&video_path_str)
            .fetch_one(&pool)
            .await?;

            // Convert bytes back to tensor (simplified)
            // Note: This is a bit hacky as we don't have direct access to internal tensor conversion from bytes in public API
            // But we can use the search feature to find distance to ITSELF if we had the vector.
            // Instead, let's just trust the process for now and maybe add a public helper if needed.
            // Actually, we can deserialize the bytes since we know it's f32 little endian

            let floats: Vec<f32> = embedding_bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_ne_bytes(chunk.try_into().unwrap()))
                .collect();

            embeddings.push((count, floats));
        }

        // Compare embeddings (Cosine Similarity)
        println!("\nSimilarity Matrix (vs 20 frames):");
        let reference = &embeddings.last().unwrap().1; // 20 frames

        for (count, emb) in &embeddings {
            let sim = cosine_similarity(emb, reference);
            println!("Frames {}: {:.4}", count, sim);
        }
    }

    let _ = std::fs::remove_file(db_path);
    Ok(())
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot_product / (norm_a * norm_b)
}
