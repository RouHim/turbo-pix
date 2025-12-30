CREATE TABLE IF NOT EXISTS video_semantic_metadata (
    path TEXT PRIMARY KEY,
    num_frames_sampled INTEGER NOT NULL,
    frame_times TEXT NOT NULL,
    model_version TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);
