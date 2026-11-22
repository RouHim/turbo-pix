-- CLIP moved from Candle/safetensors to the ONNX Runtime encoders in
-- `src/clip.rs`. The two runtimes do not produce bit-identical embeddings, so
-- every stored vector is invalidated and rebuilt by the next library scan.
DELETE FROM media_semantic_vectors;
DELETE FROM semantic_vector_path_mapping;
DELETE FROM video_semantic_metadata;
UPDATE photos SET semantic_vector_indexed = 0;
