CREATE TABLE IF NOT EXISTS semantic_vector_path_mapping (
    id INTEGER PRIMARY KEY,
    path TEXT UNIQUE NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS media_semantic_vectors
USING vec0(semantic_vector float[512]);
