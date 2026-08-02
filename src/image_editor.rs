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
use sha2::{Digest, Sha256};

use crate::cache_manager::CacheManager;
use crate::db::{DbPool, Photo};
use crate::raw_processor;

/// Check if a file is a video based on its extension
fn is_video_file(file_path: &Path) -> bool {
    const VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "mov", "avi", "mkv", "webm", "m4v"];

    file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .map(|ext| VIDEO_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

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
    PermissionDenied(String),
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
            Self::PermissionDenied(msg) => write!(f, "Permission denied: {}", msg),
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
pub async fn rotate_image(
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

    // Block video files (not supported)
    if is_video_file(file_path) {
        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown");
        return Err(ImageEditError::UnsupportedFormat(format!(
            "Video format '.{}' cannot be rotated. Video rotation is not supported.",
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

    // Capture EXIF from the ORIGINAL file before the pixel transform. The
    // `image` crate re-encodes from the pixel buffer and drops all EXIF, so
    // the full field set must be carried over explicitly afterwards (with
    // orientation forced to 1).
    let original_exif = match crate::exif_helpers::read_exif_from_path(file_path) {
        Ok(exif) => Some(exif),
        Err(e) => {
            // Continuing without EXIF would re-encode the rotated file with
            // NO EXIF — silent data loss (e.g. PNGs whose EXIF chunk carries
            // the pngext-convention "Exif\0\0" prefix fail to parse here).
            log::warn!(
                "Failed to read EXIF from {} before rotation; rotated file will be written without EXIF: {}",
                file_path.display(),
                e
            );
            None
        }
    };

    // Warn when the DB orientation disagrees with the file's EXIF: the stale
    // DB value would bake in a wrong irreversible pixel rotation below.
    if let Some(exif) = original_exif.as_ref() {
        if let Some(file_orientation) = exif
            .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .and_then(|f| f.value.get_uint(0))
        {
            if photo.orientation.is_some() && photo.orientation != Some(file_orientation as i32) {
                log::warn!(
                    "EXIF orientation {} in {} differs from DB orientation {:?}; rotating with the DB value",
                    file_orientation,
                    file_path.display(),
                    photo.orientation
                );
            }
        }
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

    // Save rotated image to temporary location first (to avoid corruption).
    // The temp name ends in the real extension so `image::save` can infer the
    // format, and carries a monotonic counter so two concurrent rotations of
    // the same photo cannot both write (and then rename away) the same temp
    // path — a deterministic collision that made the loser fail with a
    // spurious error or leave interleaved temp content.
    static ROTATE_TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temp_path = file_path.with_extension(format!(
        "tmp.{}.{}",
        ROTATE_TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        extension
    ));
    rotated_img
        .save(&temp_path)
        .map_err(|e| ImageEditError::WriteError(format!("Failed to save rotated image: {}", e)))?;

    // Carry the original EXIF (orientation reset to 1) into the rotated file.
    // The re-encoded temp file has no EXIF, so this also serves as the
    // orientation reset — a file-level Orientation = 1 marker prevents
    // viewers from double-rotating the already-rotated pixels.
    if let Err(e) =
        carry_exif_with_reset_orientation(&temp_path, &extension, original_exif.as_ref())
    {
        // Cleanup temp file
        let _ = std::fs::remove_file(&temp_path);
        return Err(ImageEditError::ExifError(format!(
            "Failed to carry EXIF into rotated image: {}",
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
    if let Err(e) = invalidate_semantic_vector(db_pool, &photo.file_path).await {
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

    // Rewriting hash_sha256 violates housekeeping_candidates' FK (no ON UPDATE);
    // drop the stale candidate row inside the same transaction (AGENTS.md known bug).
    let mut tx = db_pool.begin().await.map_err(|e| {
        ImageEditError::DatabaseError(format!("Failed to begin transaction: {}", e))
    })?;
    sqlx::query("DELETE FROM housekeeping_candidates WHERE photo_hash = ?")
        .bind(&old_hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            ImageEditError::DatabaseError(format!("Failed to delete housekeeping candidate: {}", e))
        })?;
    updated_photo
        .update_with_old_hash(&mut tx, &old_hash)
        .await
        .map_err(|e| ImageEditError::DatabaseError(format!("Failed to update database: {}", e)))?;
    tx.commit().await.map_err(|e| {
        ImageEditError::DatabaseError(format!("Failed to commit transaction: {}", e))
    })?;

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

/// Writes the original EXIF (orientation tag forced to 1) into a re-encoded
/// image file. `image::save` does not preserve EXIF, so rotation would
/// otherwise permanently strip camera/date/GPS data from the file.
fn carry_exif_with_reset_orientation(
    file_path: &Path,
    format: &str,
    original_exif: Option<&exif::Exif>,
) -> Result<(), String> {
    // No EXIF in the original — nothing to preserve
    let exif = match original_exif {
        Some(exif) => exif,
        None => return Ok(()),
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
        } else if field.ifd_num == In::THUMBNAIL {
            // The experimental EXIF writer drops JPEGInterchangeFormat/
            // Length and never receives the thumbnail JPEG blob, so carrying
            // IFD1 entries would emit a dangling/corrupt IFD1 block with
            // entries but no image data. metadata_writer deliberately skips
            // these for the same reason (should_copy_field).
            continue;
        } else if matches!(field.value, Value::Unknown(..)) {
            // The experimental EXIF writer cannot serialize unknown value
            // types; skip them rather than failing the whole rotation.
            continue;
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

    // Generate new EXIF data and write it back
    let exif_bytes = crate::exif_helpers::build_exif_buffer(&new_fields)?;
    crate::exif_helpers::write_exif_to_image(file_path, format, exif_bytes)
}

/// Computes SHA256 hash of file
fn compute_file_hash(file_path: &Path) -> Result<String, ImageEditError> {
    let file_bytes = std::fs::read(file_path).map_err(|e| {
        ImageEditError::ReadError(format!("Failed to read file for hashing: {}", e))
    })?;

    let mut hasher = Sha256::new();
    hasher.update(&file_bytes);
    let hash = hasher.finalize();

    Ok(hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>())
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
pub async fn delete_photo(
    photo: &Photo,
    db_pool: &DbPool,
    cache_manager: &CacheManager,
) -> Result<(), ImageEditError> {
    let file_path = std::path::Path::new(&photo.file_path);

    // Delete the original file
    if file_path.exists() {
        std::fs::remove_file(file_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(30) {
                return ImageEditError::PermissionDenied(
                    "Photo directory is mounted as read-only".to_string(),
                );
            }

            ImageEditError::WriteError(format!("Failed to delete file: {}", e))
        })?;
        log::info!("Deleted file: {}", photo.file_path);
    } else {
        log::warn!(
            "File not found, skipping file deletion: {}",
            photo.file_path
        );
    }

    // Delete thumbnails - spawn async task to avoid blocking
    // Thumbnails are keyed by content hash, not file path
    let photo_hash = photo.hash_sha256.clone();
    let cache_manager_clone = cache_manager.clone();
    tokio::spawn(async move {
        if let Err(e) = cache_manager_clone.clear_for_hash(&photo_hash).await {
            log::warn!("Failed to clear cache for {}: {}", photo_hash, e);
        }
    });

    // Delete semantic vector
    if let Err(e) = invalidate_semantic_vector(db_pool, &photo.file_path).await {
        log::warn!(
            "Failed to delete semantic vector for {}: {}",
            photo.file_path,
            e
        );
    }

    // Delete from database
    sqlx::query("DELETE FROM photos WHERE hash_sha256 = ?")
        .bind(&photo.hash_sha256)
        .execute(db_pool)
        .await
        .map_err(|e| {
            ImageEditError::DatabaseError(format!("Failed to delete from database: {}", e))
        })?;

    log::info!("Deleted photo from database: {}", photo.hash_sha256);

    Ok(())
}

/// Invalidates semantic vector for a file path
/// The vector will be regenerated during the next midnight rescan
async fn invalidate_semantic_vector(pool: &DbPool, file_path: &str) -> Result<(), String> {
    // Delete from mapping table
    sqlx::query("DELETE FROM semantic_vector_path_mapping WHERE path = ?")
        .bind(file_path)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete semantic vector mapping: {}", e))?;

    // Delete from video metadata if present
    sqlx::query("DELETE FROM video_semantic_metadata WHERE path = ?")
        .bind(file_path)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete video semantic metadata: {}", e))?;

    // Orphaned vectors in media_semantic_vectors will be cleaned up by the cleanup job

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_manager::CacheManager;
    use crate::db::{create_in_memory_pool, Photo};
    use chrono::Utc;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn create_test_photo(temp_dir: &TempDir, filename: &str) -> (std::path::PathBuf, Photo) {
        let source_path = Path::new("test-data/IMG_9377.jpg");
        let dest_path = temp_dir.path().join(filename);
        fs::copy(source_path, &dest_path).expect("Failed to copy test image");

        // Compute hash
        let file_bytes = fs::read(&dest_path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&file_bytes);
        let hash = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();

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

    #[tokio::test]
    async fn test_rotate_image_90() {
        // GIVEN: A test image in database
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().await.unwrap();

        let (file_path, photo) = create_test_photo(&temp_dir, "test_rotate_90.jpg");
        photo.create_or_update(&db_pool).await.unwrap();

        let original_width = photo.width.unwrap();
        let original_height = photo.height.unwrap();
        let original_hash = photo.hash_sha256.clone();

        // WHEN: Rotate image 90 degrees
        let result = rotate_image(&photo, RotationAngle::Rotate90, &db_pool).await;

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

    #[tokio::test]
    async fn test_rotate_preserves_exif_in_file() {
        // GIVEN: An EXIF-bearing image (IMG_9377.jpg has Canon EXIF) in database
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().await.unwrap();

        let (file_path, photo) = create_test_photo(&temp_dir, "test_rotate_exif.jpg");
        photo.create_or_update(&db_pool).await.unwrap();

        // Sanity: the fixture really carries EXIF
        let before =
            crate::exif_helpers::read_exif_from_path(&file_path).expect("fixture must have EXIF");
        // Make is an Ascii tag, so `get_uint` yields None for it; compare the
        // display value instead so a corrupted/emptied Make fails the test.
        let make_before = before
            .get_field(exif::Tag::Make, exif::In::PRIMARY)
            .map(|f| f.value.display_as(exif::Tag::Make).to_string());
        assert!(
            make_before.as_deref().is_some_and(|s| !s.is_empty()),
            "fixture must have a Make field"
        );
        assert!(
            before
                .get_field(exif::Tag::DateTime, exif::In::PRIMARY)
                .is_some(),
            "fixture must have a DateTime field"
        );

        // WHEN: Rotate 90 degrees
        rotate_image(&photo, RotationAngle::Rotate90, &db_pool)
            .await
            .expect("rotation should succeed");

        // THEN: The rotated file still contains the original EXIF, with the
        // orientation tag reset to 1 (regression: rotation used to re-encode
        // without EXIF, silently stripping camera/date/GPS data)
        let after = crate::exif_helpers::read_exif_from_path(&file_path)
            .expect("rotated file must still have EXIF");
        let orientation = after
            .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .and_then(|f| f.value.get_uint(0));
        assert_eq!(orientation, Some(1), "file orientation must be reset to 1");
        let make_after = after
            .get_field(exif::Tag::Make, exif::In::PRIMARY)
            .map(|f| f.value.display_as(exif::Tag::Make).to_string());
        assert_eq!(make_after, make_before, "EXIF Make must survive rotation");
        assert!(
            after
                .get_field(exif::Tag::DateTime, exif::In::PRIMARY)
                .is_some(),
            "EXIF DateTime must survive rotation"
        );
    }

    #[tokio::test]
    async fn test_rotate_image_180() {
        // GIVEN: A test image
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().await.unwrap();

        let (file_path, photo) = create_test_photo(&temp_dir, "test_rotate_180.jpg");
        photo.create_or_update(&db_pool).await.unwrap();

        let original_width = photo.width.unwrap();
        let original_height = photo.height.unwrap();

        // WHEN: Rotate 180 degrees
        let result = rotate_image(&photo, RotationAngle::Rotate180, &db_pool).await;

        // THEN: Should succeed
        assert!(result.is_ok());

        let updated_photo = result.unwrap();

        // THEN: Dimensions should be unchanged (180 rotation)
        assert_eq!(updated_photo.width.unwrap(), original_width);
        assert_eq!(updated_photo.height.unwrap(), original_height);

        // THEN: File should still be valid
        assert!(image::open(&file_path).is_ok());
    }

    #[tokio::test]
    async fn test_rotate_raw_file_blocked() {
        // GIVEN: A RAW file (if available)
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().await.unwrap();

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
        let result = rotate_image(&photo, RotationAngle::Rotate90, &db_pool).await;

        // THEN: Should fail with UnsupportedFormat error
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ImageEditError::UnsupportedFormat(_)
        ));
    }

    #[tokio::test]
    async fn test_rotate_nonexistent_file() {
        // GIVEN: Photo with nonexistent file
        let db_pool = create_in_memory_pool().await.unwrap();

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
        let result = rotate_image(&photo, RotationAngle::Rotate90, &db_pool).await;

        // THEN: Should fail with FileNotFound
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ImageEditError::FileNotFound(_)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_delete_photo_read_only_directory_returns_permission_denied() {
        // GIVEN: A photo file in a read-only directory
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().await.unwrap();
        let cache_manager = CacheManager::new(temp_dir.path().join("cache"));

        let read_only_dir = temp_dir.path().join("read_only");
        fs::create_dir_all(&read_only_dir).unwrap();
        let photo_path = read_only_dir.join("test_delete_read_only.jpg");
        fs::copy(Path::new("test-data/IMG_9377.jpg"), &photo_path).unwrap();
        fs::set_permissions(&photo_path, fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(&read_only_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let photo = Photo {
            hash_sha256: "delete-read-only-hash".to_string(),
            file_path: photo_path.to_string_lossy().to_string(),
            filename: "test_delete_read_only.jpg".to_string(),
            file_size: 12345,
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

        // WHEN: Attempt to delete
        let result = delete_photo(&photo, &db_pool, &cache_manager).await;

        // THEN: Should fail with PermissionDenied and sanitized message
        assert!(matches!(result, Err(ImageEditError::PermissionDenied(_))));

        let message = match result {
            Err(ImageEditError::PermissionDenied(msg)) => msg,
            _ => String::new(),
        };
        assert_eq!(message, "Photo directory is mounted as read-only");
        assert!(!message.contains(&photo.file_path));

        fs::set_permissions(&read_only_dir, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn test_delete_photo_writable_file_succeeds() {
        // GIVEN: A writable test photo
        let temp_dir = TempDir::new().unwrap();
        let db_pool = create_in_memory_pool().await.unwrap();
        let cache_manager = CacheManager::new(temp_dir.path().join("cache"));
        let (file_path, mut photo) = create_test_photo(&temp_dir, "test_delete_writable.jpg");
        photo.hash_sha256 = "delete-writable-hash".to_string();

        // WHEN: Delete photo
        let result = delete_photo(&photo, &db_pool, &cache_manager).await;

        // THEN: Should succeed and remove file
        assert!(result.is_ok(), "Delete failed: {:?}", result);
        assert!(!file_path.exists());
    }
}
