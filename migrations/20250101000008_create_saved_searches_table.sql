CREATE TABLE IF NOT EXISTS saved_searches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    query TEXT,
    view TEXT NOT NULL DEFAULT 'all',
    sort TEXT NOT NULL DEFAULT 'date_desc',
    year INTEGER,
    month INTEGER,
    created_at TEXT DEFAULT (datetime('now'))
);

-- State identity: exact duplicate view states are one entry (COALESCE so
-- NULL query/year/month participate in uniqueness; SQLite unique indexes
-- treat NULLs as distinct).
CREATE UNIQUE INDEX IF NOT EXISTS idx_saved_searches_state
    ON saved_searches (COALESCE(query, ''), view, sort, COALESCE(year, 0), COALESCE(month, 0));
