//! Model download and ONNX Runtime sessions.
//!
//! The weights are non-commercial InsightFace models. They are deliberately
//! fetched at runtime (never committed or bundled) from pinned HuggingFace
//! revisions; the SHA-256 checks below are the local integrity gate.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ort::environment::Environment;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

use crate::model_files::verify_sha256;
use crate::ort_runtime::shared_environment;

pub const MODEL_VERSION: &str = "scrfd-2.5g_antelopev2-r100-v2";

const DETECTOR_REPO: (&str, &str) = ("immich-app", "buffalo_m");
const DETECTOR_REVISION: &str = "b95a1bc582b2e58f9555673bdcfd5e14e1fbdc61";
const DETECTOR_FILE: &str = "detection/model.onnx";
const DETECTOR_SHA256: &str = "041f73f47371333d1d17a6fee6c8ab4e6aecabefe398ff32cca4e2d5eaee0af9";

const RECOGNIZER_REPO: (&str, &str) = ("immich-app", "antelopev2");
const RECOGNIZER_REVISION: &str = "51ca7e153bccebdbae224779e23c9c243e2fec85";
const RECOGNIZER_FILE: &str = "recognition/model.onnx";
const RECOGNIZER_SHA256: &str = "4ab1d6435d639628a6f3e5008dd4f929edf4c4124b1a7169e1048f9fef534cdf";

#[derive(Clone, Debug)]
pub struct FaceModelPaths {
    pub detector: PathBuf,
    pub recognizer: PathBuf,
}

pub struct FaceModels {
    pub detector: Session,
    pub recognizer: Session,
    /// Shared process-wide environment kept alive for both sessions.
    _environment: Environment,
}

/// Resolves (and, on a cold cache, downloads) the two pinned model files.
pub fn resolve_model_paths(data_path: &str) -> Result<FaceModelPaths> {
    // Same `..` convention as CLIP: the E2E suite points TURBO_PIX_DATA_PATH at
    // a sandbox but shares the repo's ./data/models cache.
    let cache_dir = Path::new(data_path).join("../data/models/face");

    // Air-gapped/test fallback: the two files may be placed in the cache by
    // hand under stable names. The hashes are checked exactly like a download.
    let local_detector = cache_dir.join("detector.onnx");
    let local_recognizer = cache_dir.join("recognizer.onnx");
    if local_detector.is_file() && local_recognizer.is_file() {
        verify_sha256(&local_detector, DETECTOR_SHA256)?;
        verify_sha256(&local_recognizer, RECOGNIZER_SHA256)?;
        return Ok(FaceModelPaths {
            detector: local_detector,
            recognizer: local_recognizer,
        });
    }

    let client = hf_hub::HFClient::builder()
        .cache_dir(cache_dir)
        .build_sync()
        .context("Failed to build HuggingFace API for face models")?;

    let detector = client
        .model(DETECTOR_REPO.0, DETECTOR_REPO.1)
        .download_file()
        .filename(DETECTOR_FILE)
        .revision(DETECTOR_REVISION)
        .send()
        .context("Failed to download the SCRFD detector")?;
    verify_sha256(&detector, DETECTOR_SHA256)?;

    let recognizer = client
        .model(RECOGNIZER_REPO.0, RECOGNIZER_REPO.1)
        .download_file()
        .filename(RECOGNIZER_FILE)
        .revision(RECOGNIZER_REVISION)
        .send()
        .context("Failed to download the ArcFace recognizer")?;
    verify_sha256(&recognizer, RECOGNIZER_SHA256)?;

    Ok(FaceModelPaths {
        detector,
        recognizer,
    })
}

/// Downloads the face models explicitly (`--download-models`).
pub fn download_face_models(data_path: &str) -> Result<()> {
    let paths = resolve_model_paths(data_path)?;
    log::info!("Face detector ready: {}", paths.detector.display());
    log::info!("Face recognizer ready: {}", paths.recognizer.display());
    Ok(())
}

/// Loads both sessions. The caller decides whether this runs on a blocking
/// thread; session creation and the first ORT initialization are synchronous.
pub fn load_models(paths: &FaceModelPaths, threads: usize) -> Result<FaceModels> {
    let environment = shared_environment()?;
    let threads = threads.max(1);

    let detector = Session::builder(&environment)
        .map_err(|error| anyhow::anyhow!("ORT detector builder: {error}"))?
        .with_flush_to_zero()
        .map_err(|error| anyhow::anyhow!("ORT detector flush-to-zero: {error}"))?
        .with_intra_threads(threads)
        .map_err(|error| anyhow::anyhow!("ORT detector threads: {error}"))?
        .commit_from_file(&paths.detector)
        .map_err(|error| anyhow::anyhow!("ORT detector load: {error}"))?;

    // Level 3's NCHWc layout transform costs 25% on this model (712 ms vs
    // 533 ms for one 112x112 inference); level 2 keeps the Conv+BN fusions.
    // The detector is the opposite case and must stay on the default level.
    //
    // Flush-to-zero is the bigger lever here: ArcFace's activations reach the
    // subnormal range and cost 4x on x86 without it (528 ms -> 131 ms).
    // Every session sets it, not just this one: ORT applies the flag to the
    // process-wide MXCSR from the first session created, and each session's
    // own thread pool applies it from its own options.
    let recognizer = Session::builder(&environment)
        .map_err(|error| anyhow::anyhow!("ORT recognizer builder: {error}"))?
        .with_optimization_level(GraphOptimizationLevel::Level2)
        .map_err(|error| anyhow::anyhow!("ORT recognizer optimization level: {error}"))?
        .with_flush_to_zero()
        .map_err(|error| anyhow::anyhow!("ORT recognizer flush-to-zero: {error}"))?
        .with_intra_threads(threads)
        .map_err(|error| anyhow::anyhow!("ORT recognizer threads: {error}"))?
        .commit_from_file(&paths.recognizer)
        .map_err(|error| anyhow::anyhow!("ORT recognizer load: {error}"))?;

    Ok(FaceModels {
        detector,
        recognizer,
        _environment: environment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_version_is_stable() {
        assert_eq!(MODEL_VERSION, "scrfd-2.5g_antelopev2-r100-v2");
    }

    #[test]
    fn checksum_rejects_a_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.onnx");
        std::fs::write(&path, b"not the model").unwrap();

        assert!(verify_sha256(&path, DETECTOR_SHA256).is_err());
    }
}
