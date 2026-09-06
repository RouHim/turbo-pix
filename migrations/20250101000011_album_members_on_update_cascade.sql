-- Rekey-safe album memberships: rotating a photo rewrites
-- photos.hash_sha256, which album_members.photo_hash references. Without
-- ON UPDATE CASCADE that parent-key rewrite fails with
-- FOREIGN KEY constraint failed whenever the photo belongs to an album.
-- Rebuild the child table with the cascade so hash rewrites repoint
-- memberships automatically (code also repoints explicitly inside the
-- same rotate/rekey transactions as belt-and-braces).
CREATE TABLE album_members_new (
    album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    photo_hash TEXT NOT NULL REFERENCES photos(hash_sha256) ON DELETE CASCADE ON UPDATE CASCADE,
    added_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (album_id, photo_hash)
);
INSERT OR IGNORE INTO album_members_new (album_id, photo_hash, added_at)
    SELECT album_id, photo_hash, added_at FROM album_members;
DROP TABLE album_members;
ALTER TABLE album_members_new RENAME TO album_members;
CREATE INDEX IF NOT EXISTS idx_album_members_photo ON album_members(photo_hash);
