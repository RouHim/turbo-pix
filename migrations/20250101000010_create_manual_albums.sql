CREATE TABLE IF NOT EXISTS albums (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS album_members (
    album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    photo_hash TEXT NOT NULL REFERENCES photos(hash_sha256) ON DELETE CASCADE,
    added_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (album_id, photo_hash)
);
CREATE INDEX IF NOT EXISTS idx_album_members_photo ON album_members(photo_hash);
DROP TABLE IF EXISTS event_albums;
