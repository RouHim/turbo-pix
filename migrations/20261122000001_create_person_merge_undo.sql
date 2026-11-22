-- One persisted merge-undo slot plus its snapshots.
--
-- Face ids are deleted and re-inserted on every re-index, so the snapshot is
-- keyed by (photo_hash, face_index) and copies person names/hidden flags by
-- value: an undo can re-create the source person without depending on any
-- row that a rescan replaces.

CREATE TABLE IF NOT EXISTS person_merge_undo (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS person_merge_undo_source (
    undo_id INTEGER NOT NULL REFERENCES person_merge_undo(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL,
    name TEXT,
    hidden INTEGER NOT NULL,
    PRIMARY KEY (undo_id, person_id)
);

CREATE TABLE IF NOT EXISTS person_merge_undo_face (
    undo_id INTEGER NOT NULL REFERENCES person_merge_undo(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL,
    photo_hash TEXT NOT NULL,
    face_index INTEGER NOT NULL,
    PRIMARY KEY (undo_id, photo_hash, face_index)
);
