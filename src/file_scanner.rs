use chrono::{DateTime, Utc};
use log::{info, warn};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Hard cap on directory nesting depth. Prevents pathological directory
/// structures from recursing until stack overflow (belt-and-braces next to
/// the canonical-path cycle detection).
const MAX_SCAN_DEPTH: usize = 64;

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

    /// Scans all configured photo paths.
    ///
    /// Returns the found files plus a `scan_complete` flag that is `false`
    /// whenever any root was missing or any directory could not be read.
    /// Callers must skip orphan cleanup on a partial scan — deleting rows for
    /// files that are merely temporarily unreachable (unmounted drive,
    /// permission change) would permanently lose favorites and manual
    /// metadata edits.
    pub fn scan(&self) -> (Vec<PhotoFile>, bool) {
        let mut photos = Vec::new();
        let mut scan_complete = true;
        let mut visited_dirs = HashSet::new();

        for root_path in &self.photo_paths {
            if !root_path.exists() {
                warn!("Photo directory does not exist: {}", root_path.display());
                scan_complete = false;
                continue;
            }

            info!("Scanning directory: {}", root_path.display());

            Self::walk_directory(
                root_path,
                &mut photos,
                &mut visited_dirs,
                0,
                &mut scan_complete,
            );
        }

        info!("Found {} photos", photos.len());
        (photos, scan_complete)
    }

    /// Recursively walk a directory and collect photo files.
    ///
    /// Cycle-safe: every directory is canonicalized and recorded in
    /// `visited_dirs`, so a symlink loop (`current -> ..`) is detected and
    /// skipped instead of recursing until stack overflow (which aborts the
    /// whole server process mid-rescan). Symlinked directories are still
    /// walked (canonicalization makes that safe); symlinked files are
    /// indexed like regular files.
    fn walk_directory(
        dir: &Path,
        photos: &mut Vec<PhotoFile>,
        visited_dirs: &mut HashSet<PathBuf>,
        depth: usize,
        scan_complete: &mut bool,
    ) {
        if depth > MAX_SCAN_DEPTH {
            warn!(
                "Directory nesting exceeds {} levels at {} — skipping",
                MAX_SCAN_DEPTH,
                dir.display()
            );
            *scan_complete = false;
            return;
        }

        let canonical = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        if !visited_dirs.insert(canonical) {
            // Already walked through another path — symlink cycle detected.
            return;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Cannot read directory {}: {}", dir.display(), e);
                *scan_complete = false;
                return;
            }
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };

            if file_type.is_dir() {
                if Self::is_collage_dir(&path) {
                    continue;
                }
                Self::walk_directory(&path, photos, visited_dirs, depth + 1, scan_complete);
            } else if file_type.is_file() && Self::is_supported_file(&path) {
                Self::push_photo(&path, photos);
            } else if file_type.is_symlink() {
                // Follow symlinks (dirs are cycle-protected above, files are
                // indexed as usual), but never treat the symlink itself as a
                // directory entry.
                if let Ok(metadata) = fs::metadata(&path) {
                    if metadata.is_dir() {
                        if Self::is_collage_dir(&path) {
                            continue;
                        }
                        Self::walk_directory(&path, photos, visited_dirs, depth + 1, scan_complete);
                    } else if metadata.is_file() && Self::is_supported_file(&path) {
                        Self::push_photo(&path, photos);
                    }
                }
            }
        }
    }

    fn push_photo(path: &Path, photos: &mut Vec<PhotoFile>) {
        if let Ok(metadata) = fs::metadata(path) {
            photos.push(PhotoFile {
                path: path.to_path_buf(),
                size: metadata.len(),
                modified: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| {
                        DateTime::from_timestamp(duration.as_secs() as i64, 0)
                            .unwrap_or_else(Utc::now)
                    }),
                metadata,
            });
        }
    }

    fn is_collage_dir(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name == "collages")
            .unwrap_or(false)
    }

    fn is_supported_file(path: &Path) -> bool {
        let supported_extensions = [
            // Standard image formats
            "jpg", "jpeg", "png", "tiff", "tif", "bmp", "webp", // Video formats
            "mp4", "mov", "avi", "mkv", "webm", "m4v", // RAW formats
            "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "dng", "pef",
        ];

        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| supported_extensions.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"test").unwrap();
        path
    }

    #[test]
    fn scan_collects_photos_recursively() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        write_test_file(root, "a.jpg");
        write_test_file(&sub, "b.png");

        let scanner = FileScanner::new(vec![root.to_path_buf()]);
        let (photos, scan_complete) = scanner.scan();

        assert!(scan_complete);
        let mut names: Vec<_> = photos
            .iter()
            .map(|p| p.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.jpg", "b.png"]);
    }

    #[test]
    fn scan_ignores_collage_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let collages = root.join("collages");
        std::fs::create_dir_all(&collages).unwrap();
        write_test_file(&collages, "c.jpg");

        let scanner = FileScanner::new(vec![root.to_path_buf()]);
        let (photos, _) = scanner.scan();
        assert!(photos.is_empty());
    }

    #[test]
    fn scan_survives_symlink_cycles() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write_test_file(root, "a.jpg");
        // `loop -> .` creates a symlink cycle that would recurse forever
        // without canonical-path cycle detection.
        std::os::unix::fs::symlink(root, root.join("loop")).unwrap();
        // Chain link as well: loop2 -> loop
        std::os::unix::fs::symlink(root.join("loop"), root.join("loop2")).unwrap();

        let scanner = FileScanner::new(vec![root.to_path_buf()]);
        let (photos, scan_complete) = scanner.scan();

        assert!(scan_complete);
        assert_eq!(photos.len(), 1);
        assert_eq!(
            photos[0].path.file_name().unwrap().to_string_lossy(),
            "a.jpg"
        );
    }

    #[test]
    fn scan_reports_incomplete_when_root_missing() {
        let temp = tempfile::tempdir().unwrap();
        let scanner = FileScanner::new(vec![temp.path().join("missing")]);
        let (photos, scan_complete) = scanner.scan();
        assert!(photos.is_empty());
        assert!(!scan_complete);
    }

    #[test]
    fn scan_reports_incomplete_when_subdirectory_unreadable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        write_test_file(root, "a.jpg");
        // Make the subdirectory unreadable so read_dir fails on it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        }
        // Restore permissions so the tempdir can be cleaned up.
        let result = FileScanner::new(vec![root.to_path_buf()]).scan();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(result.0.len(), 1);
        assert!(!result.1);
    }
}
