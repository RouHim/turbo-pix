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

    pub async fn clear_for_path(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        info!("Clearing cache for deleted photo: {}", path);

        // Clear thumbnail files that might exist for this path
        let filename = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        info!("Looking for thumbnails with filename: {}", filename);
        info!(
            "Thumbnail cache directory: {}",
            self.thumbnail_cache_dir.display()
        );

        for size in ["small", "medium", "large"] {
            for format in ["jpeg", "webp"] {
                let thumbnail_path = self
                    .thumbnail_cache_dir
                    .join(format!("{}_{}.{}", filename, size, format));
                info!("Checking thumbnail path: {}", thumbnail_path.display());
                if thumbnail_path.exists() {
                    if let Err(e) = std::fs::remove_file(&thumbnail_path) {
                        error!(
                            "Failed to remove thumbnail {}: {}",
                            thumbnail_path.display(),
                            e
                        );
                    } else {
                        info!("Removed thumbnail: {}", thumbnail_path.display());
                    }
                } else {
                    info!("Thumbnail does not exist: {}", thumbnail_path.display());
                }
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
