pub const CREATE_PHOTOS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS photos (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL,
    filename TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    mime_type TEXT NOT NULL,
    date_taken DATETIME,
    date_modified DATETIME NOT NULL,
    date_indexed DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    width INTEGER,
    height INTEGER,
    orientation INTEGER DEFAULT 1,
    camera_make TEXT,
    camera_model TEXT,
    iso INTEGER,
    aperture REAL,
    shutter_speed TEXT,
    focal_length REAL,
    gps_latitude REAL,
    gps_longitude REAL,
    location_name TEXT,
    hash_md5 TEXT,
    hash_sha256 TEXT UNIQUE,
    thumbnail_path TEXT,
    has_thumbnail BOOLEAN DEFAULT FALSE
);
"#;

pub const CREATE_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_photos_date_taken ON photos(date_taken);
CREATE INDEX IF NOT EXISTS idx_photos_path ON photos(path);
CREATE INDEX IF NOT EXISTS idx_photos_hash_md5 ON photos(hash_md5);
CREATE INDEX IF NOT EXISTS idx_photos_hash_sha256 ON photos(hash_sha256);
CREATE INDEX IF NOT EXISTS idx_photos_location ON photos(gps_latitude, gps_longitude);
"#;
