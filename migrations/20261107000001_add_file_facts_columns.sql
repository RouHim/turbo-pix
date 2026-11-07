-- The photo file is the authority for its capture date and coordinates; the
-- database keeps a derived copy for ordering, filtering and map placement.
--
-- The copy is read-only to every path that does not read the file: the scan's
-- upsert and the two metadata saves are its only writers, and both derive it
-- from a fresh read of the file (file first, row second). The scan reconciles
-- it on every pass — including the unchanged-file fast path — so an edit made
-- outside the app heals at the next scan.
--
-- `taken_at` stays a zero-padded RFC3339 UTC string so text comparison is
-- chronological; the composite index serves the date-ordered listing in both
-- directions and the month range (`taken_at >= 'YYYY-MM' AND taken_at < 'YYYY-MM'`).
ALTER TABLE photos ADD COLUMN taken_at TEXT;
ALTER TABLE photos ADD COLUMN taken_latitude REAL;
ALTER TABLE photos ADD COLUMN taken_longitude REAL;

CREATE INDEX IF NOT EXISTS idx_photos_taken_at ON photos(taken_at, hash_sha256);
