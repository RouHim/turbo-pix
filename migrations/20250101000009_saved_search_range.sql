-- Range bounds for saved searches: `year`/`month` are the start bound,
-- `to_year`/`to_month` the inclusive end bound (NULL = single period).
ALTER TABLE saved_searches ADD COLUMN to_year INTEGER;
ALTER TABLE saved_searches ADD COLUMN to_month INTEGER;

-- State identity now includes the end bound; a range is a distinct entry from
-- its start-period twin. COALESCE so NULL bounds participate in uniqueness.
DROP INDEX IF EXISTS idx_saved_searches_state;
CREATE UNIQUE INDEX IF NOT EXISTS idx_saved_searches_state
    ON saved_searches (
        COALESCE(query, ''),
        view,
        sort,
        COALESCE(year, 0),
        COALESCE(month, 0),
        COALESCE(to_year, 0),
        COALESCE(to_month, 0)
    );
