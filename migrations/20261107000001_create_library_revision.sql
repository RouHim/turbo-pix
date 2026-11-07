-- The listing order is derived: capture dates live in the photo files, so the
-- in-memory order cache cannot be invalidated by a date column. What it can be
-- invalidated by is a change to the photo row set — this counter row is bumped
-- by the triggers below and read once per listing, so a cached order can never
-- outlive an insert, a delete or an in-app re-key (rotation rewrites
-- `hash_sha256` in place, the path stays).
CREATE TABLE IF NOT EXISTS library_revision (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO library_revision (id, revision) VALUES (1, 0);

CREATE TRIGGER IF NOT EXISTS photos_revision_insert
AFTER INSERT ON photos
BEGIN
    UPDATE library_revision SET revision = revision + 1 WHERE id = 1;
END;

CREATE TRIGGER IF NOT EXISTS photos_revision_delete
AFTER DELETE ON photos
BEGIN
    UPDATE library_revision SET revision = revision + 1 WHERE id = 1;
END;

-- The scan's UPSERT names `file_path` in its SET list even when the value is
-- unchanged, so the value comparison (never the column list alone) decides.
CREATE TRIGGER IF NOT EXISTS photos_revision_rekey
AFTER UPDATE OF hash_sha256, file_path ON photos
WHEN OLD.hash_sha256 != NEW.hash_sha256 OR OLD.file_path != NEW.file_path
BEGIN
    UPDATE library_revision SET revision = revision + 1 WHERE id = 1;
END;
