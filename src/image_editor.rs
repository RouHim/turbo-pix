//! Image Editor Module
//!
//! Provides image transformation operations (rotation, mirroring) with:
//! - Physical pixel transformation using `image` crate
//! - EXIF orientation reset to 1 (standard)
//! - RAW file format protection
//! - File hash recomputation
//! - Thumbnail cache invalidation

use std::path::Path;

use exif::{Field, In, Tag, Value};
use image::GenericImageView;
use img_parts::jpeg::Jpeg;
use img_parts::png::Png;
use img_parts::{Bytes, ImageEXIF};
use sha2::{Digest, Sha256};

use crate::cache_manager::CacheManager;
use crate::db::{DbPool, Photo};
use crate::raw_processor;

/// Angle for rotation operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationAngle {
    Rotate90,
    Rotate180,
    Rotate270,
}

/// Error types for image editing operations
#[derive(Debug)]
pub enum ImageEditError {
    UnsupportedFormat(String),
    FileNotFound(String),
    ReadError(String),
    WriteError(String),
    ExifError(String),
    DatabaseError(String),
}

impl std::fmt::Display for ImageEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat(msg) => write!(f, "Unsupported format: {}", msg),
            Self::FileNotFound(msg) => write!(f, "File not found: {}", msg),
            Self::ReadError(msg) => write!(f, "Read error: {}", msg),
            Self::WriteError(msg) => write!(f, "Write error: {}", msg),
            Self::ExifError(msg) => write!(f, "EXIF error: {}", msg),
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for ImageEditError {}

/// Rotates an image file by the specified angle
///
/// This performs a TRUE rotation:
/// 1. Loads the image and applies physical pixel transformation
/// 2. Resets EXIF orientation tag to 1 (standard orientation)
/// 3. Recomputes file SHA256 hash
/// 4. Invalidates thumbnail cache
/// 5. Invalidates semantic vector (will be regenerated at midnight rescan)
/// 6. Updates database with new hash and dimensions
///
/// # Arguments
/// * `photo` - Photo entity from database
/// * `angle` - Rotation angle (90, 180, or 270 degrees clockwise)
/// * `db_pool` - Database connection pool
///
/// # Returns
/// Updated Photo entity with new hash and dimensions
///
/// # Note
/// Thumbnails are invalidated by setting `has_thumbnail = false` in the database.
/// The cache manager is not needed as orphaned thumbnails are cleaned up separately.
pub fn rotate_image(
    photo: &Photo,
    angle: RotationAngle,
    db_pool: &DbPool,
) -> Result<Photo, ImageEditError> {
    let file_path = Path::new(&photo.file_path);

    // Validate file exists
    if !file_path.exists() {
        return Err(ImageEditError::FileNotFound(format!(
            "File not found: {}",
            photo.file_path
        )));
    }

    // Block RAW files (cannot write EXIF changes)
    if raw_processor::is_raw_file(file_path) {
        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown");
        return Err(ImageEditError::UnsupportedFormat(format!(
            "RAW format '.{}' cannot be rotated. RAW files are read-only. Convert to JPEG/PNG first.",
            extension
        )));
    }

    // Validate format (JPEG/PNG only)
    let extension = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .ok_or_else(|| ImageEditError::UnsupportedFormat("File has no extension".to_string()))?;

    if !["jpg", "jpeg", "png"].contains(&extension.as_str()) {
        return Err(ImageEditError::UnsupportedFormat(format!(
            "Format '.{}' is not supported for rotation. Only JPEG and PNG are supported.",
            extension
        )));
    }

    // Load image
    let mut img = image::open(file_path)
        .map_err(|e| ImageEditError::ReadError(format!("Failed to load image: {}", e)))?;

    // Apply existing EXIF orientation to pixels first
    // This ensures we're rotating the actual visual orientation, not the stored orientation
    if let Some(orientation) = photo.orientation {
        img = match orientation {
            2 => img.fliph(),
            3 => img.rotate180(),
            4 => img.flipv(),
            5 => img.fliph().rotate270(),
            6 => img.rotate90(),
            7 => img.fliph().rotate90(),
            8 => img.rotate270(),
            _ => img, // 1 or unknown = no transformation
        };
    }

    // Now apply the requested rotation
    let rotated_img = match angle {
        RotationAngle::Rotate90 => img.rotate90(),
        RotationAngle::Rotate180 => img.rotate180(),
        RotationAngle::Rotate270 => img.rotate270(),
    };

    // Save rotated image to temporary location first (to avoid corruption)
    let temp_path = file_path.with_extension(format!("tmp.{}", extension));
    rotated_img
        .save(&temp_path)
        .map_err(|e| ImageEditError::WriteError(format!("Failed to save rotated image: {}", e)))?;

    // Reset EXIF orientation to 1 (standard orientation)
    if let Err(e) = reset_exif_orientation(&temp_path, &extension) {
        // Cleanup temp file
        let _ = std::fs::remove_file(&temp_path);
        return Err(ImageEditError::ExifError(format!(
            "Failed to reset EXIF orientation: {}",
            e
        )));
    }

    // Replace original file with rotated version
    std::fs::rename(&temp_path, file_path).map_err(|e| {
        // Cleanup temp file if rename fails
        let _ = std::fs::remove_file(&temp_path);
        ImageEditError::WriteError(format!("Failed to replace original file: {}", e))
    })?;

    // Recompute file hash (content changed)
    let new_hash = compute_file_hash(file_path)?;

    // Get new dimensions
    let (new_width, new_height) = rotated_img.dimensions();

    // Invalidate thumbnail cache (async operation, but we'll do it sync for simplicity)
    // The old thumbnails will become orphaned when hash changes - they'll be cleaned up later
    // We could spawn a task here, but it's not critical for correctness

    // Invalidate semantic vector (will be regenerated at midnight rescan)
    if let Err(e) = invalidate_semantic_vector(db_pool, &photo.file_path) {
        log::warn!(
            "Failed to invalidate semantic vector for {}: {}",
            photo.file_path,
            e
        );
    }

    // Store old hash before updating
    let old_hash = photo.hash_sha256.clone();

    // Update photo in database
    let mut updated_photo = photo.clone();
    updated_photo.hash_sha256 = new_hash;
    updated_photo.width = Some(new_width as i32);
    updated_photo.height = Some(new_height as i32);
    updated_photo.orientation = Some(1); // Reset to standard orientation
    updated_photo.has_thumbnail = Some(false); // Thumbnails invalidated
    updated_photo.semantic_vector_indexed = Some(false); // Semantic vector invalidated
    updated_photo.updated_at = chrono::Utc::now();

    // Use update_with_old_hash to find record by old hash and update to new hash
    updated_photo
        .update_with_old_hash(db_pool, &old_hash)
        .map_err(|e| ImageEditError::DatabaseError(format!("Failed to update database: {}", e)))?;

    log::info!(
        "Rotated image {:?}: {} -> {} ({}x{} -> {}x{})",
        angle,
        photo.hash_sha256,
        updated_photo.hash_sha256,
        photo.width.unwrap_or(0),
        photo.height.unwrap_or(0),
        new_width,
        new_height
    );

    Ok(updated_photo)
}

/// Resets EXIF orientation tag to 1 (standard orientation)
fn reset_exif_orientation(file_path: &Path, format: &str) -> Result<(), String> {
    // Read existing EXIF data
    let file = std::fs::File::open(file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mut bufreader = std::io::BufReader::new(&file);
    let exifreader = exif::Reader::new();

    let exif = match exifreader.read_from_container(&mut bufreader) {
        Ok(exif) => exif,
        Err(_) => {
            // No EXIF data - nothing to reset
            return Ok(());
        }
    };

    // Collect all fields, setting orientation to 1
    let mut new_fields: Vec<Field> = Vec::new();

    for field in exif.fields() {
        if field.tag == Tag::Orientation {
            // Replace with orientation = 1
            new_fields.push(Field {
                tag: Tag::Orientation,
                ifd_num: In::PRIMARY,
                value: Value::Short(vec![1]),
            });
        } else {
            // Keep existing field
            new_fields.push(Field {
                tag: field.tag,
                ifd_num: field.ifd_num,
                value: field.value.clone(),
            });
        }
    }

    // If no orientation tag existed, add one
    if !new_fields.iter().any(|f| f.tag == Tag::Orientation) {
        new_fields.push(Field {
            tag: Tag::Orientation,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![1]),
        });
    }

    // Generate new EXIF data
    let mut exif_buffer = std::io::Cursor::new(Vec::new());
    let mut writer = exif::experimental::Writer::new();

    for field in &new_fields {
        writer.push_field(field);
    }

    writer
        .write(&mut exif_buffer, false)
        .map_err(|e| format!("Failed to generate EXIF data: {}", e))?;

    let exif_bytes = Bytes::from(exif_buffer.into_inner());

    // Write EXIF based on format
    match format {
        "jpg" | "jpeg" => {
            let image_bytes =
                std::fs::read(file_path).map_err(|e| format!("Failed to read JPEG: {}", e))?;

            let mut jpeg = Jpeg::from_bytes(image_bytes.into())
                .map_err(|e| format!("Failed to parse JPEG: {}", e))?;

            jpeg.set_exif(Some(exif_bytes));

            let output_bytes = jpeg.encoder().bytes();
            std::fs::write(file_path, output_bytes)
                .map_err(|e| format!("Failed to write JPEG: {}", e))?;
        }
        "png" => {
            let image_bytes =
                std::fs::read(file_path).map_err(|e| format!("Failed to read PNG: {}", e))?;

            let mut png = Png::from_bytes(image_bytes.into())
                .map_err(|e| format!("Failed to parse PNG: {}", e))?;

            png.set_exif(Some(exif_bytes));

            let output_bytes = png.encoder().bytes();
            std::fs::write(file_path, output_bytes)
                .map_err(|e| format!("Failed to write PNG: {}", e))?;
        }
        _ => return Err(format!("Unsupported format: {}", format)),
    }

    Ok(())
}

/// Computes SHA256 hash of file
fn compute_file_hash(file_path: &Path) -> Result<String, ImageEditError> {
    let file_bytes = std::fs::read(file_path).map_err(|e| {
        ImageEditError::ReadError(format!("Failed to read file for hashing: {}", e))
    })?;

    let mut hasher = Sha256::new();
    hasher.update(&file_bytes);
    let hash = hasher.finalize();

    Ok(format!("{:x}", hash))
}

/// Deletes a photo file and all associated data
///
/// This performs complete deletion:
/// 1. Deletes the original file from disk
/// 2. Removes database record
/// 3. Deletes all thumbnails
/// 4. Removes semantic vector
///
/// # Arguments
/// * `photo` - Photo entity to delete
/// * `db_pool` - Database connection pool
/// * `cache_manager` - Cache manager for thumbnail deletion
///
/// # Returns
/// Ok(()) on success, error otherwise
pub fn delete_photo(
    photo: &Photo,
    db_pool: &DbPool,
    cache_manager: &CacheManager,
) -> Result<(), ImageEditError> {
    let file_path = std::path::Path::new(&photo.file_path);

    // Delete the original file
    if file_path.exists() {
        std::fs::remove_file(file_path)
            .map_err(|e| ImageEditError::WriteError(format!("Failed to delete file: {}", e)))?;
        log::info!("Deleted file: {}", photo.file_path);
    } else {
        log::warn!(
            "File not found, skipping file deletion: {}",
            photo.file_path
        );
    }

    // Delete thumbnails - spawn async task to avoid blocking
    // Clear cache for this photo's path
    let file_path_clone = photo.file_path.clone();
    let cache_manager_clone = cache_manager.clone();
    tokio::spawn(async move {
        if let Err(e) = cache_manager_clone.clear_for_path(&file_path_clone).await {
            log::warn!("Failed to clear cache for {}: {}", file_path_clone, e);
        }
    });

    // Delete semantic vector
    if let Err(e) = invalidate_semantic_vector(db_pool, &photo.file_path) {
        log::warn!(
            "Failed to delete semantic vector for {}: {}",
            photo.file_path,
            e
        );
    }

    // Delete from database
    let conn = db_pool.get().map_err(|e| {
        ImageEditError::DatabaseError(format!("Failed to get DB connection: {}", e))
    })?;

    conn.execute(
        "DELETE FROM photos WHERE hash_sha256 = ?",
        [&photo.hash_sha256],
    )
    .map_err(|e| ImageEditError::DatabaseError(format!("Failed to delete from database: {}", e)))?;

    log::info!("Deleted photo from database: {}", photo.hash_sha256);

    Ok(())
}

/// Invalidates semantic vector for a file path
/// The vector will be regenerated during the next midnight rescan
fn invalidate_semantic_vector(pool: &DbPool, file_path: &str) -> Result<(), String> {
    let conn = pool
        .get()
        .map_err(|e| format!("Failed to get database connection: {}", e))?;

    // Delete from mapping table
    conn.execute(
        "DELETE FROM semantic_vector_path_mapping WHERE path = ?",
        [file_path],
    )
    .map_err(|e| format!("Failed to delete semantic vector mapping: {}", e))?;

    // Delete from video metadata if present
    conn.execute(
        "DELETE FROM video_semantic_metadata WHERE path = ?",
        [file_path],
    )
    .map_err(|e| format!("Failed to delete video semantic metadata: {}", e))?;

    // Orphaned vectors in media_semantic_vectors will be cleaned up by the cleanup job

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_in_memory_pool, Photo};
    use chrono::Utc;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_photo(temp_dir: &TempDir, filename: &str) -> (std::path::PathBuf, Photo) {
        // Copy test image to temp directory
        let source_path = Path::new("test-data/sample_with_exif.jpg");
        let dest_path = temp_dir.path().join(filename);
        fs::copy(source_path, &dest_path).expect("Failed to copy test image");

        // Compute hash
        let file_bytes = fs::read(&dest_path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&file_bytes);
        let hash = format!("{:x}", hasher.finalize());

        // Get dimensions
        let img = image::open(&dest_path).unwrap();
        let (width, height) = img.dimensions();

        let photo = Photo {
            hash_sha256: hash,
            file_path: dest_path.to_string_lossy().to_string(),
            filename: filename.to_string(),
            file_size: file_bytes.len() as i64,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            width: Some(width as i32),
            height: Some(height as i32),
            orientation: Some(1),
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: serde_json::json!({}),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        (dest_path, photo)
    }

    #[test]
    fn test_rotate_image_90() {
        // GIVEN: A test image in database
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().unwrap();

        let (file_path, photo) = create_test_photo(&temp_dir, "test_rotate_90.jpg");
        photo.create_or_update(&db_pool).unwrap();

        let original_width = photo.width.unwrap();
        let original_height = photo.height.unwrap();
        let original_hash = photo.hash_sha256.clone();

        // WHEN: Rotate image 90 degrees
        let result = rotate_image(&photo, RotationAngle::Rotate90, &db_pool);

        // THEN: Should succeed
        assert!(result.is_ok(), "Rotation failed: {:?}", result);

        let updated_photo = result.unwrap();

        // THEN: Dimensions should be swapped (90 degree rotation)
        assert_eq!(updated_photo.width.unwrap(), original_height);
        assert_eq!(updated_photo.height.unwrap(), original_width);

        // THEN: Hash should be different (file content changed)
        assert_ne!(updated_photo.hash_sha256, original_hash);

        // THEN: Orientation should be reset to 1
        assert_eq!(updated_photo.orientation, Some(1));

        // THEN: Thumbnails invalidated
        assert_eq!(updated_photo.has_thumbnail, Some(false));

        // THEN: Semantic vector invalidated
        assert_eq!(updated_photo.semantic_vector_indexed, Some(false));

        // THEN: File should still be a valid image
        assert!(image::open(&file_path).is_ok());
    }

    #[test]
    fn test_rotate_image_180() {
        // GIVEN: A test image
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().unwrap();

        let (file_path, photo) = create_test_photo(&temp_dir, "test_rotate_180.jpg");
        photo.create_or_update(&db_pool).unwrap();

        let original_width = photo.width.unwrap();
        let original_height = photo.height.unwrap();

        // WHEN: Rotate 180 degrees
        let result = rotate_image(&photo, RotationAngle::Rotate180, &db_pool);

        // THEN: Should succeed
        assert!(result.is_ok());

        let updated_photo = result.unwrap();

        // THEN: Dimensions should be unchanged (180 rotation)
        assert_eq!(updated_photo.width.unwrap(), original_width);
        assert_eq!(updated_photo.height.unwrap(), original_height);

        // THEN: File should still be valid
        assert!(image::open(&file_path).is_ok());
    }

    #[test]
    fn test_rotate_raw_file_blocked() {
        // GIVEN: A RAW file (if available)
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().unwrap();

        let raw_source = Path::new("test-data/IMG_9899.CR2");
        if !raw_source.exists() {
            return; // Skip test if RAW file not available
        }

        let raw_dest = temp_dir.path().join("test.CR2");
        fs::copy(raw_source, &raw_dest).unwrap();

        let photo = Photo {
            hash_sha256: "test_hash".to_string(),
            file_path: raw_dest.to_string_lossy().to_string(),
            filename: "test.CR2".to_string(),
            file_size: 1024,
            mime_type: Some("image/x-canon-cr2".to_string()),
            taken_at: None,
            width: Some(800),
            height: Some(600),
            orientation: Some(1),
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: serde_json::json!({}),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // WHEN: Attempt to rotate RAW file
        let result = rotate_image(&photo, RotationAngle::Rotate90, &db_pool);

        // THEN: Should fail with UnsupportedFormat error
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ImageEditError::UnsupportedFormat(_)
        ));
    }

    #[test]
    fn test_rotate_nonexistent_file() {
        // GIVEN: Photo with nonexistent file
        let db_pool = create_in_memory_pool().unwrap();

        let photo = Photo {
            hash_sha256: "test".to_string(),
            file_path: "/nonexistent/file.jpg".to_string(),
            filename: "file.jpg".to_string(),
            file_size: 1024,
            mime_type: Some("image/jpeg".to_string()),
            taken_at: None,
            width: Some(800),
            height: Some(600),
            orientation: Some(1),
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: serde_json::json!({}),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // WHEN: Attempt to rotate
        let result = rotate_image(&photo, RotationAngle::Rotate90, &db_pool);

        // THEN: Should fail with FileNotFound
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ImageEditError::FileNotFound(_)
        ));
    }
}
