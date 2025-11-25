use chrono::{DateTime, Duration, NaiveDate, Utc};
use image::{DynamicImage, ImageBuffer, Rgba, RgbaImage};
use log::{error, info};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::db::Photo;
use crate::db_pool::DbPool;
use crate::file_scanner::PhotoFile;
use crate::photo_processor::PhotoProcessor;

/// Collage entity representing a generated photo collage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collage {
    pub id: i64,
    pub date: String, // Format: YYYY-MM-DD
    pub file_path: String,
    pub thumbnail_path: Option<String>,
    pub photo_count: i32,
    pub photo_hashes: Vec<String>, // JSON array of hashes
    pub accepted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Collage {
    /// Parse from SQLite row
    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        let photo_hashes_json: String = row.get(5)?;
        let photo_hashes: Vec<String> =
            serde_json::from_str(&photo_hashes_json).unwrap_or_default();

        Ok(Collage {
            id: row.get(0)?,
            date: row.get(1)?,
            file_path: row.get(2)?,
            thumbnail_path: row.get(3)?,
            photo_count: row.get(4)?,
            photo_hashes,
            accepted_at: row
                .get::<_, Option<String>>(6)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            created_at: row
                .get::<_, Option<String>>(7)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
        })
    }

    /// List all pending collages
    pub fn list_pending(pool: &DbPool) -> Result<Vec<Self>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, date, file_path, thumbnail_path, photo_count, photo_hashes,
                    accepted_at, created_at
             FROM collages
             WHERE accepted_at IS NULL
             ORDER BY date DESC",
        )?;

        let collages = stmt
            .query_map([], Self::from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(collages)
    }

    /// Get collage by ID
    pub fn get_by_id(pool: &DbPool, id: i64) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, date, file_path, thumbnail_path, photo_count, photo_hashes,
                    accepted_at, created_at
             FROM collages
             WHERE id = ?",
        )?;

        match stmt.query_row([id], Self::from_row) {
            Ok(collage) => Ok(Some(collage)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    /// Check if collage exists for date
    /// Insert new collage
    pub fn insert(
        pool: &DbPool,
        date: &str,
        file_path: &str,
        thumbnail_path: Option<&str>,
        photo_count: i32,
        photo_hashes: &[String],
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let photo_hashes_json = serde_json::to_string(photo_hashes)?;

        conn.execute(
            "INSERT INTO collages (date, file_path, thumbnail_path, photo_count, photo_hashes)
             VALUES (?, ?, ?, ?, ?)",
            params![
                date,
                file_path,
                thumbnail_path,
                photo_count,
                photo_hashes_json
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Mark collage as accepted and update file path
    pub fn accept(
        pool: &DbPool,
        id: i64,
        new_file_path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE collages SET accepted_at = CURRENT_TIMESTAMP, file_path = ? WHERE id = ?",
            [new_file_path, &id.to_string()],
        )?;
        Ok(())
    }

    /// Delete collage
    pub fn delete(pool: &DbPool, id: i64) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute("DELETE FROM collages WHERE id = ?", [id])?;
        Ok(())
    }
}

/// Photo cluster representing photos taken on the same day
#[derive(Debug)]
struct PhotoCluster {
    date: NaiveDate,
    photos: Vec<Photo>,
}

const MAX_PHOTOS_PER_COLLAGE: usize = 4;

/// Collage layout configuration
struct CollageLayout {
    grid_cols: usize,
    photo_count: usize,
    cell_width: u32,
    cell_height: u32,
}

impl CollageLayout {
    /// Calculate optimal grid layout for photo count
    fn calculate(photo_count: usize) -> Self {
        let clamped = photo_count.clamp(1, MAX_PHOTOS_PER_COLLAGE);

        // Max 2x2 grid to keep each tile large and readable
        let (grid_rows, grid_cols) = match clamped {
            1 => (1, 1),
            2 => (1, 2),
            _ => (2, 2),
        };

        // 4K resolution (3840x2160) divided by grid
        let cell_width = 3840 / grid_cols as u32;
        let cell_height = 2160 / grid_rows as u32;

        CollageLayout {
            grid_cols,
            photo_count: clamped,
            cell_width,
            cell_height,
        }
    }
}

/// Find photo clusters (dates with ≥10 photos) in the last 365 days
fn find_photo_clusters(pool: &DbPool) -> Result<Vec<PhotoCluster>, Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    // Get cutoff date (365 days ago)
    let cutoff_date = (Utc::now() - Duration::days(365)).to_rfc3339();

    // Find dates with ≥10 photos
    let mut stmt = conn.prepare(
        "SELECT DATE(taken_at) as photo_date, COUNT(*) as count
         FROM photos
         WHERE taken_at IS NOT NULL
           AND taken_at >= ?
         GROUP BY photo_date
         HAVING count >= 10
         ORDER BY photo_date DESC",
    )?;

    let dates: Vec<String> = stmt
        .query_map([&cutoff_date], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut clusters = Vec::new();

    for date_str in dates {
        // Parse date
        let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")?;

        // Get all photos for this date
        let mut photo_stmt = conn.prepare(
            "SELECT * FROM photos
             WHERE DATE(taken_at) = ?
             ORDER BY taken_at",
        )?;

        let photos = photo_stmt
            .query_map([&date_str], Photo::from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        if photos.len() >= 10 {
            clusters.push(PhotoCluster { date, photos });
        }
    }

    Ok(clusters)
}

/// Create collage image from photos
fn create_collage_image(
    photos: &[&Photo],
    layout: &CollageLayout,
) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    // Create 4K canvas (3840x2160)
    let mut canvas: RgbaImage = ImageBuffer::from_pixel(3840, 2160, Rgba([0, 0, 0, 255]));

    for (idx, photo) in photos.iter().take(layout.photo_count).enumerate() {
        let row = idx / layout.grid_cols;
        let col = idx % layout.grid_cols;

        // Load and resize image
        let img = match image::open(&photo.file_path) {
            Ok(img) => img,
            Err(e) => {
                error!("Failed to load image {}: {}", photo.file_path, e);
                continue;
            }
        };

        // Resize to fit cell while maintaining aspect ratio
        let resized = img.resize_to_fill(
            layout.cell_width,
            layout.cell_height,
            image::imageops::FilterType::Lanczos3,
        );

        // Calculate position on canvas
        let x_offset = col as u32 * layout.cell_width;
        let y_offset = row as u32 * layout.cell_height;

        // Paste image onto canvas
        image::imageops::overlay(
            &mut canvas,
            &resized.to_rgba8(),
            x_offset as i64,
            y_offset as i64,
        );
    }

    Ok(canvas)
}

fn chunk_photos<'a>(photos: &'a [Photo]) -> Vec<Vec<&'a Photo>> {
    if photos.is_empty() {
        return Vec::new();
    }

    // Always aim for two collages, or three when more than 12 photos are present.
    let target_collages = if photos.len() > 12 { 3 } else { 2 };
    let max_slots = target_collages * MAX_PHOTOS_PER_COLLAGE;

    // Distribute photos round-robin while respecting per-collage cap (4).
    let mut buckets: Vec<Vec<&Photo>> = vec![Vec::new(); target_collages];
    let mut filled = 0;

    for (idx, photo) in photos.iter().enumerate() {
        if filled >= max_slots {
            break;
        }

        let mut bucket_idx = idx % target_collages;
        let mut attempts = 0;
        while buckets[bucket_idx].len() >= MAX_PHOTOS_PER_COLLAGE && attempts < target_collages {
            bucket_idx = (bucket_idx + 1) % target_collages;
            attempts += 1;
        }

        if buckets[bucket_idx].len() < MAX_PHOTOS_PER_COLLAGE {
            buckets[bucket_idx].push(photo);
            filled += 1;
        }
    }

    // Only keep fully populated collages to avoid empty tiles.
    buckets
        .into_iter()
        .filter(|b| b.len() == MAX_PHOTOS_PER_COLLAGE)
        .collect()
}

/// Generate collages for all detected clusters
pub async fn generate_collages(
    pool: &DbPool,
    data_path: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    info!("Starting collage generation...");

    // Create staging directory
    let staging_dir = data_path.join("collages").join("staging");
    std::fs::create_dir_all(&staging_dir)?;

    // Find clusters
    let clusters = find_photo_clusters(pool)?;
    info!("Found {} photo clusters to process", clusters.len());

    let mut generated_count = 0;

    for cluster in clusters {
        let date_str = cluster.date.format("%Y-%m-%d").to_string();
        let chunks = chunk_photos(&cluster.photos);

        if chunks.is_empty() {
            info!(
                "No photos found for {}; skipping collage generation",
                date_str
            );
            continue;
        }

        info!(
            "Generating {} collages for {} ({} photos total)",
            chunks.len(),
            date_str,
            cluster.photos.len()
        );

        for (collage_idx, chunk) in chunks.iter().enumerate() {
            // Calculate layout for the current chunk (max 2x2)
            let layout = CollageLayout::calculate(chunk.len());

            // Create collage image
            let collage_img = match create_collage_image(chunk, &layout) {
                Ok(img) => img,
                Err(e) => {
                    error!(
                        "Failed to create collage {} for {}: {}",
                        collage_idx + 1,
                        date_str,
                        e
                    );
                    continue;
                }
            };

            // Save collage
            let filename = format!("collage_{}_{}.jpg", date_str, collage_idx + 1);
            let file_path = staging_dir.join(&filename);
            let img = DynamicImage::ImageRgba8(collage_img);

            if let Err(e) = img.save_with_format(&file_path, image::ImageFormat::Jpeg) {
                error!("Failed to save collage to {:?}: {}", file_path, e);
                continue;
            }

            // For now, skip thumbnail generation for collages
            // Thumbnails can be generated on-demand later if needed
            let thumbnail_path: Option<String> = None;

            // Save to database
            let photo_hashes: Vec<String> = chunk.iter().map(|p| p.hash_sha256.clone()).collect();

            match Collage::insert(
                pool,
                &date_str,
                &file_path.to_string_lossy(),
                thumbnail_path.as_deref(),
                chunk.len() as i32,
                &photo_hashes,
            ) {
                Ok(_) => {
                    info!(
                        "Successfully created collage {} for {}",
                        collage_idx + 1,
                        date_str
                    );
                    generated_count += 1;
                }
                Err(e) => {
                    error!("Failed to insert collage into database: {}", e);
                    // Clean up file
                    let _ = std::fs::remove_file(&file_path);
                }
            }
        }
    }

    info!(
        "Collage generation complete: {} collages created",
        generated_count
    );
    Ok(generated_count)
}

/// Move accepted collage to photos directory and trigger indexing
pub async fn accept_collage(
    pool: &DbPool,
    collage_id: i64,
    data_path: &Path,
    semantic_search: std::sync::Arc<crate::semantic_search::SemanticSearchEngine>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Get collage
    let collage = Collage::get_by_id(pool, collage_id)?.ok_or("Collage not found")?;

    // Create destination directory (separate from staging to avoid premature indexing)
    let dest_dir = data_path.join("collages").join("accepted");
    std::fs::create_dir_all(&dest_dir)?;

    // Move file
    let source = PathBuf::from(&collage.file_path);
    let filename = source.file_name().ok_or("Invalid file path")?;
    let dest = dest_dir.join(filename);

    std::fs::rename(&source, &dest)?;

    // Move thumbnail if exists
    if let Some(thumb_path) = &collage.thumbnail_path {
        let thumb_source = PathBuf::from(thumb_path);
        if thumb_source.exists() {
            if let Some(thumb_filename) = thumb_source.file_name() {
                let thumb_dest = dest_dir.join(thumb_filename);
                let _ = std::fs::rename(&thumb_source, &thumb_dest);
            }
        }
    }

    // Mark as accepted and update file path
    Collage::accept(pool, collage_id, &dest.to_string_lossy())?;

    // Index the collage into photos table immediately
    if let Err(e) = index_collage_file(pool, &dest, semantic_search).await {
        error!("Failed to index collage into photos table: {}", e);
        // Don't fail the whole operation if indexing fails
    }

    Ok(dest)
}

/// Reject and delete collage
pub async fn reject_collage(
    pool: &DbPool,
    collage_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get collage
    let collage = Collage::get_by_id(pool, collage_id)?.ok_or("Collage not found")?;

    // Delete files
    let file_path = PathBuf::from(&collage.file_path);
    if file_path.exists() {
        std::fs::remove_file(&file_path)?;
    }

    if let Some(thumb_path) = &collage.thumbnail_path {
        let thumb_file = PathBuf::from(thumb_path);
        if thumb_file.exists() {
            let _ = std::fs::remove_file(&thumb_file);
        }
    }

    // Delete from database
    Collage::delete(pool, collage_id)?;

    Ok(())
}

/// Index a single collage file into the photos table
async fn index_collage_file(
    pool: &DbPool,
    file_path: &Path,
    semantic_search: std::sync::Arc<crate::semantic_search::SemanticSearchEngine>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get file metadata
    let metadata = fs::metadata(file_path)?;
    let size = metadata.len();
    let modified = metadata.modified().ok().map(|t| {
        let duration = t
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0));
        DateTime::from_timestamp(duration.as_secs() as i64, 0).unwrap_or_else(Utc::now)
    });

    // Create PhotoFile
    let photo_file = PhotoFile {
        path: file_path.to_path_buf(),
        size,
        modified,
        metadata,
    };

    // Process the file
    let processor = PhotoProcessor::new(Vec::new(), semantic_search);
    let processed_photo = processor
        .process_file_metadata_only(&photo_file)
        .await
        .ok_or("Failed to process collage file")?;

    // Convert to Photo and insert into database
    let photo: Photo = processed_photo.into();
    let mut conn = pool.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    photo.create_or_update_with_connection(&tx)?;
    tx.commit()?;

    info!("Collage indexed into photos table: {}", file_path.display());
    Ok(())
}
