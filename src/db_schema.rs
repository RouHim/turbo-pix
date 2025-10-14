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
// so this table maintains the path-to-rowid mapping for the image_semantic_vectors table.
pub const SEMANTIC_VECTOR_PATH_MAPPING_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS semantic_vector_path_mapping (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL
)
"#;

pub const IMAGE_SEMANTIC_VECTORS_TABLE: &str =
    "CREATE VIRTUAL TABLE IF NOT EXISTS image_semantic_vectors USING vec0(semantic_vector float[512])";

pub const SCHEMA_SQL: &[&str] = &[
    PHOTOS_TABLE,
    "CREATE INDEX IF NOT EXISTS idx_photos_file_path ON photos(file_path);",
    "CREATE INDEX IF NOT EXISTS idx_photos_taken_at ON photos(taken_at);",
    "CREATE INDEX IF NOT EXISTS idx_photos_mime_type ON photos(mime_type);",
    "CREATE INDEX IF NOT EXISTS idx_photos_is_favorite ON photos(is_favorite);",
    IMAGE_SEMANTIC_VECTORS_TABLE,
    SEMANTIC_VECTOR_PATH_MAPPING_TABLE,
];

pub fn initialize_schema(conn: &Connection) -> SqlResult<()> {
    for sql in SCHEMA_SQL {
        conn.execute(sql, [])?;
    }
    Ok(())
}
