-- The listing's sort field is client-selectable; `filename` and `file_size`
-- had no index, so the planner scanned the whole table and built a temp
-- b-tree for the first page of every such listing (85 ms / 886 ms at 200k
-- rows on a warm cache). The composite form serves the
-- `ORDER BY <field> <dir>, hash_sha256 <dir>` tiebreak from the index, so
-- pagination stays deterministic without a sort step.
CREATE INDEX IF NOT EXISTS idx_photos_filename ON photos(filename, hash_sha256);
CREATE INDEX IF NOT EXISTS idx_photos_file_size ON photos(file_size, hash_sha256);
