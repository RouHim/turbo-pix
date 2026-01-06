CREATE TABLE IF NOT EXISTS housekeeping_candidates (
    photo_hash TEXT NOT NULL,
    reason TEXT NOT NULL,
    score REAL NOT NULL,
    PRIMARY KEY (photo_hash),
    FOREIGN KEY (photo_hash) REFERENCES photos(hash_sha256) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_housekeeping_candidates_reason ON housekeeping_candidates(reason);
