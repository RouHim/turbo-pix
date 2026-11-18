-- Face recognition: detected faces, their embeddings and the people clusters.
--
-- `photos` is WITHOUT ROWID with `hash_sha256` as its primary key, so the
-- foreign keys below target that column. `face_vectors` is a sqlite-vec
-- virtual table: its rows are not covered by SQLite's foreign-key cascade, so
-- `face_vector_mapping` carries the cascade from `faces` and the face phase
-- removes vector rows whose mapping is gone.

CREATE TABLE IF NOT EXISTS people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    hidden INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS faces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_hash TEXT NOT NULL,
    face_index INTEGER NOT NULL,
    person_id INTEGER,
    -- Normalised coordinates (0..1) relative to the analysed thumbnail, so a
    -- re-index can match old and new faces without knowing the pixel size.
    bbox_x REAL NOT NULL,
    bbox_y REAL NOT NULL,
    bbox_w REAL NOT NULL,
    bbox_h REAL NOT NULL,
    quality REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (photo_hash) REFERENCES photos(hash_sha256) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (person_id) REFERENCES people(id) ON DELETE SET NULL,
    UNIQUE (photo_hash, face_index)
);

CREATE INDEX IF NOT EXISTS idx_faces_photo_hash ON faces(photo_hash);
CREATE INDEX IF NOT EXISTS idx_faces_person_id ON faces(person_id);

CREATE VIRTUAL TABLE IF NOT EXISTS face_vectors
USING vec0(face_vector float[512]);

CREATE TABLE IF NOT EXISTS face_vector_mapping (
    id INTEGER PRIMARY KEY,
    face_id INTEGER UNIQUE NOT NULL REFERENCES faces(id) ON DELETE CASCADE
);

-- The fingerprint is the photo row's current file facts plus the model
-- version: a changed file or a new model version makes the photo pending
-- without requiring the scan to know anything about faces.
CREATE TABLE IF NOT EXISTS face_index_state (
    photo_hash TEXT PRIMARY KEY,
    model_version TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    file_modified TEXT NOT NULL,
    indexed_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (photo_hash) REFERENCES photos(hash_sha256) ON DELETE CASCADE ON UPDATE CASCADE
);
