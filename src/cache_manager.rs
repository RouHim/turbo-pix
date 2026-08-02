use log::{error, info};
use std::path::PathBuf;

#[derive(Clone)]
pub struct CacheManager {
    thumbnail_cache_dir: PathBuf,
}

impl CacheManager {
    pub fn new(thumbnail_cache_dir: PathBuf) -> Self {
        Self {
            thumbnail_cache_dir,
        }
    }

    /// Removes all cached thumbnails for a photo hash.
    ///
    /// Thumbnails live at `{cache_dir}/{hash[..3]}/{hash}_{size}.{format}`
    /// (ThumbnailGenerator::get_cache_path), keyed by content hash. The
    /// previous implementation deleted flat `{stem}_{size}.{format}` files
    /// that the hash-based layout never produces, so orphan cleanup and
    /// rotation left stale thumbnails on disk forever (and the LRU index
    /// never saw them, so the cache grew without bound across restarts).
    ///
    /// Thumbnail filenames are `{hash}[_{version}]_{size}.{format}` (the
    /// version folds in file size + mtime so in-place edits invalidate the
    /// cache), so every file whose name starts with `{hash}_` is removed —
    /// not just the unversioned form.
    pub async fn clear_for_hash(&self, hash: &str) -> Result<(), Box<dyn std::error::Error>> {
        info!("Clearing cache for photo hash: {}", hash);

        let subdir = if hash.len() >= 3 { &hash[..3] } else { hash };
        let hash_dir = self.thumbnail_cache_dir.join(subdir);

        let prefix = format!("{}_", hash);
        if let Ok(entries) = std::fs::read_dir(&hash_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                    continue;
                };
                if !name.starts_with(&prefix) {
                    continue;
                }
                if let Err(e) = std::fs::remove_file(&path) {
                    error!("Failed to remove thumbnail {}: {}", path.display(), e);
                } else {
                    info!("Removed thumbnail: {}", path.display());
                }
            }
        }

        // Remove the now-empty hash subdirectory.
        if hash_dir.exists() {
            if let Err(e) = std::fs::remove_dir(&hash_dir) {
                // Only a non-empty dir fails here; stale entries from other
                // hashes are legitimate, so this is not an error.
                log::debug!("Left cache subdirectory {}: {}", hash_dir.display(), e);
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub async fn clear_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Clearing all cache data");

        // Clear thumbnail cache directory
        if self.thumbnail_cache_dir.exists() {
            let entries = std::fs::read_dir(&self.thumbnail_cache_dir)?;
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    if let Err(e) = std::fs::remove_file(entry.path()) {
                        error!(
                            "Failed to remove cache file {}: {}",
                            entry.path().display(),
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_file(dir: &std::path::Path, rel: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"thumb").unwrap();
    }

    #[tokio::test]
    async fn clear_for_hash_removes_hash_layout_thumbnails() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CacheManager::new(temp.path().to_path_buf());
        let hash = "abcdef0123456789";

        // GIVEN thumbnails in the real hash-based layout, INCLUDING
        // versioned names ({hash}_{version}_{size}.{format} — the version
        // folds in file size + mtime, so a changed file generates a new
        // filename and clear_for_hash must remove every version)
        for size in ["small", "medium", "large"] {
            for format in ["jpeg", "webp"] {
                create_test_file(
                    temp.path(),
                    &format!("{}/{}_{}.{}", &hash[..3], hash, size, format),
                );
                create_test_file(
                    temp.path(),
                    &format!("{}/{}_123_456_{}.{}", &hash[..3], hash, size, format),
                );
            }
        }
        // A different hash must survive
        create_test_file(temp.path(), &format!("xyz/{}_small.jpeg", "xyz".repeat(8)));

        // WHEN the cache is cleared for this hash
        manager.clear_for_hash(hash).await.unwrap();

        // THEN all twelve thumbnails (six unversioned + six versioned) and
        // the subdirectory are gone, and the unrelated hash's entry is
        // untouched
        for size in ["small", "medium", "large"] {
            for format in ["jpeg", "webp"] {
                assert!(
                    !temp
                        .path()
                        .join(&hash[..3])
                        .join(format!("{}_{}.{}", hash, size, format))
                        .exists(),
                    "thumbnail should be removed"
                );
                assert!(
                    !temp
                        .path()
                        .join(&hash[..3])
                        .join(format!("{}_123_456_{}.{}", hash, size, format))
                        .exists(),
                    "versioned thumbnail should be removed"
                );
            }
        }
        assert!(
            !temp.path().join(&hash[..3]).exists(),
            "subdir should be removed"
        );
        assert!(temp
            .path()
            .join("xyz")
            .join(format!("{}_small.jpeg", "xyz".repeat(8)))
            .exists());
    }

    #[tokio::test]
    async fn clear_for_hash_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CacheManager::new(temp.path().to_path_buf());
        // No thumbnails exist — must not error
        manager.clear_for_hash("deadbeef").await.unwrap();
        manager.clear_for_hash("ab").await.unwrap(); // short-hash path
    }
}
