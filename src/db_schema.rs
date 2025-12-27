use rusqlite::{Connection, Result as SqlResult};

// Schema definitions
pub const PHOTOS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS photos (
    -- Core identification
    hash_sha256 TEXT PRIMARY KEY NOT NULL CHECK(length(hash_sha256) = 64),
    file_path TEXT NOT NULL UNIQUE,
    filename TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    mime_type TEXT,

    -- Computational fields (used in application logic)
    taken_at DATETIME,
    width INTEGER,
    height INTEGER,
    orientation INTEGER,
    duration REAL,

    -- UI state
    thumbnail_path TEXT,
    has_thumbnail BOOLEAN DEFAULT FALSE,
    blurhash TEXT,
    is_favorite BOOLEAN DEFAULT FALSE,
    semantic_vector_indexed BOOLEAN DEFAULT FALSE,

    -- Metadata (JSON blob for all EXIF/camera/location/video metadata)
    metadata TEXT NOT NULL DEFAULT '{}',

    -- System timestamps
    file_modified DATETIME NOT NULL,
    date_indexed DATETIME,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
) WITHOUT ROWID;
"#;

// Bridge table that maps file paths to semantic vector IDs.
// SQLite's vec0 extension only supports vectors and implicit rowids,
// so this table maintains the path-to-rowid mapping for the media_semantic_vectors table.
pub const SEMANTIC_VECTOR_PATH_MAPPING_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS semantic_vector_path_mapping (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL
)
"#;

pub const MEDIA_SEMANTIC_VECTORS_TABLE: &str =
    "CREATE VIRTUAL TABLE IF NOT EXISTS media_semantic_vectors USING vec0(semantic_vector float[512])";

// Metadata table for video semantic vector computation
pub const VIDEO_SEMANTIC_METADATA_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS video_semantic_metadata (
    path TEXT PRIMARY KEY,
    num_frames_sampled INTEGER NOT NULL,
    frame_times TEXT NOT NULL,
    model_version TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
)
"#;

pub const COLLAGES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS collages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    file_path TEXT NOT NULL,
    thumbnail_path TEXT,
    photo_count INTEGER NOT NULL,
    photo_hashes TEXT NOT NULL,
    signature TEXT NOT NULL,
    accepted_at DATETIME,
    rejected_at DATETIME,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
)
"#;



pub const CLEANUP_CANDIDATES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS cleanup_candidates (
    photo_hash TEXT NOT NULL,
    reason TEXT NOT NULL,
    score REAL NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (photo_hash, reason),
    FOREIGN KEY (photo_hash) REFERENCES photos(hash_sha256) ON DELETE CASCADE
)
"#;

pub const SCHEMA_SQL: &[&str] = &[
    PHOTOS_TABLE,
    "CREATE INDEX IF NOT EXISTS idx_photos_file_path ON photos(file_path);",
    "CREATE INDEX IF NOT EXISTS idx_photos_taken_at ON photos(taken_at);",
    "CREATE INDEX IF NOT EXISTS idx_photos_mime_type ON photos(mime_type);",
    "CREATE INDEX IF NOT EXISTS idx_photos_is_favorite ON photos(is_favorite);",
    MEDIA_SEMANTIC_VECTORS_TABLE,
    SEMANTIC_VECTOR_PATH_MAPPING_TABLE,
    VIDEO_SEMANTIC_METADATA_TABLE,
    COLLAGES_TABLE,
    "CREATE INDEX IF NOT EXISTS idx_collages_date ON collages(date);",
    "CREATE INDEX IF NOT EXISTS idx_collages_accepted_at ON collages(accepted_at);",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_collages_signature ON collages(signature);",
    CLEANUP_CANDIDATES_TABLE,
    "CREATE INDEX IF NOT EXISTS idx_cleanup_candidates_reason ON cleanup_candidates(reason);",
];

pub fn initialize_schema(conn: &Connection) -> SqlResult<()> {
    for sql in SCHEMA_SQL {
        conn.execute(sql, [])?;
    }
    Ok(())
}
