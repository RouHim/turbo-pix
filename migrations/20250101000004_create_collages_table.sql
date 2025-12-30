CREATE TABLE IF NOT EXISTS collages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    file_path TEXT NOT NULL,
    thumbnail_path TEXT,
    photo_count INTEGER NOT NULL,
    photo_hashes TEXT NOT NULL,
    signature TEXT NOT NULL,
    accepted_at TEXT,
    rejected_at TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);
