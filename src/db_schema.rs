use rusqlite::{Connection, Result as SqlResult};

// Schema definitions
pub const PHOTOS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS photos (
    hash_sha256 TEXT PRIMARY KEY NOT NULL CHECK(length(hash_sha256) = 64),
    file_path TEXT NOT NULL UNIQUE,
    filename TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    mime_type TEXT,
    taken_at DATETIME,
    file_modified DATETIME NOT NULL,
    date_indexed DATETIME,
    camera_make TEXT,
    camera_model TEXT,
    lens_make TEXT,
    lens_model TEXT,
    iso INTEGER,
    aperture REAL,
    shutter_speed TEXT,
    focal_length REAL,
    width INTEGER,
    height INTEGER,
    color_space TEXT,
    white_balance TEXT,
    exposure_mode TEXT,
    metering_mode TEXT,
    orientation INTEGER,
    flash_used BOOLEAN,
    latitude REAL,
    longitude REAL,
    location_name TEXT,
    thumbnail_path TEXT,
    has_thumbnail BOOLEAN,
    country TEXT,
    keywords TEXT,
    faces_detected TEXT,
    objects_detected TEXT,
    colors TEXT,
    duration REAL, -- Video duration in seconds
    video_codec TEXT, -- Video codec (e.g., "h264", "h265")
    audio_codec TEXT, -- Audio codec (e.g., "aac", "mp3")
    bitrate INTEGER, -- Bitrate in kbps
    frame_rate REAL, -- Frame rate for videos
    is_favorite BOOLEAN DEFAULT FALSE,
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
    "CREATE INDEX IF NOT EXISTS idx_photos_camera_make ON photos(camera_make);",
    "CREATE INDEX IF NOT EXISTS idx_photos_camera_model ON photos(camera_model);",
    "CREATE INDEX IF NOT EXISTS idx_photos_file_modified ON photos(file_modified);",
    "CREATE INDEX IF NOT EXISTS idx_photos_keywords ON photos(keywords);",
    "CREATE INDEX IF NOT EXISTS idx_photos_faces_detected ON photos(faces_detected);",
    "CREATE INDEX IF NOT EXISTS idx_photos_objects_detected ON photos(objects_detected);",
    "CREATE INDEX IF NOT EXISTS idx_photos_colors ON photos(colors);",
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
