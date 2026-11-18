//! Face detection, alignment and recognition.
//!
//! The detector is SCRFD-2.5G and the recognizer is antelopev2's R100
//! (512-dimensional ArcFace embedding). Both are ONNX models executed by the
//! pinned static ONNX Runtime; the runtime archive is supplied at build time by
//! `scripts/fetch-ort.sh`.

pub mod detector;
pub mod engine;
pub mod geometry;
pub mod model;
pub mod pipeline;
pub mod store;

pub use pipeline::{FaceRecognition, FaceRecognitionEngine, NoopFaceRecognition};
