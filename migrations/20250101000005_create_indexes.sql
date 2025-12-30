CREATE INDEX IF NOT EXISTS idx_photos_file_path ON photos(file_path);
CREATE INDEX IF NOT EXISTS idx_photos_taken_at ON photos(taken_at);
CREATE INDEX IF NOT EXISTS idx_photos_mime_type ON photos(mime_type);
CREATE INDEX IF NOT EXISTS idx_photos_is_favorite ON photos(is_favorite);

CREATE INDEX IF NOT EXISTS idx_collages_date ON collages(date);
CREATE INDEX IF NOT EXISTS idx_collages_accepted_at ON collages(accepted_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_collages_signature ON collages(signature);
