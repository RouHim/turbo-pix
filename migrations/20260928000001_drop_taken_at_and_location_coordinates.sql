-- Capture dates and coordinates live in the photo files only (spec
-- .spec/remove-bulk-date-shift-and-file-only-dates.md, FR-001). The stored
-- copies were never authoritative: they are dropped, not migrated.
DROP INDEX IF EXISTS idx_photos_taken_at;
ALTER TABLE photos DROP COLUMN taken_at;
UPDATE photos SET metadata = json_remove(metadata, '$.location.latitude', '$.location.longitude');
