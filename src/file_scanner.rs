use chrono::{DateTime, Utc};
use log::{info, warn};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PhotoFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub metadata: std::fs::Metadata,
}

pub struct FileScanner {
    photo_paths: Vec<PathBuf>,
}

impl FileScanner {
    pub fn new(photo_paths: Vec<PathBuf>) -> Self {
        Self { photo_paths }
    }

    pub fn scan(&self) -> Vec<PhotoFile> {
        let mut photos = Vec::new();

        for root_path in &self.photo_paths {
            if !root_path.exists() {
                warn!("Photo directory does not exist: {}", root_path.display());
                continue;
            }

            info!("Scanning directory: {}", root_path.display());

            Self::walk_directory(root_path, &mut photos);
        }

        info!("Found {} photos", photos.len());
        photos
    }

    /// Recursively walk a directory and collect photo files
    fn walk_directory(dir: &Path, photos: &mut Vec<PhotoFile>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();

                if path.is_dir() {
                    Self::walk_directory(&path, photos);
                } else if path.is_file() && Self::is_supported_file(&path) {
                    if let Ok(metadata) = fs::metadata(&path) {
                        photos.push(PhotoFile {
                            path: path.clone(),
                            size: metadata.len(),
                            modified: metadata
                                .modified()
                                .ok()
                                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|duration| {
                                    DateTime::from_timestamp(duration.as_secs() as i64, 0)
                                        .unwrap_or_else(Utc::now)
                                }),
                            metadata: metadata.clone(),
                        });
                    }
                }
            }
        }
    }

    fn is_supported_file(path: &Path) -> bool {
        let supported_extensions = [
            "jpg", "jpeg", "png", "tiff", "tif", "bmp", "webp", "mp4", "mov", "avi", "mkv", "webm",
            "m4v",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| supported_extensions.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }
}
