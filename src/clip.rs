//! CLIP ViT-B/32 ONNX encoders for semantic search.
//!
//! The text and vision towers are pinned exports of
//! `openai/clip-vit-base-patch32`, downloaded on first use and verified by
//! SHA-256. Both run through ONNX Runtime — the same runtime as the face
//! pipeline — so TurboPix has a single inference backend.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use anyhow::{Context, Result};
use ort::session::{Session, SessionOutputs};
use ort::value::{Shape, Tensor, TensorElementType, ValueType};
use tokenizers::Tokenizer;

use crate::model_files::verify_sha256;
use crate::ort_runtime::{model_threads, rss_label, rss_mib, shared_environment, HeapTrim};
use crate::raw_processor;

/// Bumped whenever the embeddings stop being binary-compatible. Video
/// metadata stores it; the migration that accompanies this change resets the
/// photo flags so the next scan re-embeds the library.
pub(crate) const MODEL_VERSION: &str = "clip-vit-base-patch32-onnx-v2";

const REPO: (&str, &str) = ("Xenova", "clip-vit-base-patch32");
const REVISION: &str = "6b7c523703ae26e39be4b03774966f8571c7f523";

const VISION_FILE: &str = "onnx/vision_model.onnx";
const VISION_SHA256: &str = "fd6e1402a588279d1723c7534d4bcba5bc0b14b47dfab0e46f8c47b8270d7d40";
const TEXT_FILE: &str = "onnx/text_model.onnx";
const TEXT_SHA256: &str = "3f6571f5bad13a97c469c1622e1cfc4d9aef78b79fdbfcff804ca357bfada8cc";
const TOKENIZER_FILE: &str = "tokenizer.json";
const TOKENIZER_SHA256: &str = "f7f3b7af117d467b58374797691a6438d3e6b9e9cef800dfd5dced7f697a90cd";

const VISION_INPUT: &str = "pixel_values";
const VISION_OUTPUT: &str = "image_embeds";
const TEXT_INPUT: &str = "input_ids";
const TEXT_OUTPUT: &str = "text_embeds";

pub(crate) const EMBEDDING_DIM: usize = 512;

// Text encoding parameters (match the pinned CLIP tokenizer).
const CONTEXT_LENGTH: usize = 77;
const EOT_TOKEN: u32 = 49407;

// Image preprocessing constants from the CLIP feature extractor.
const IMAGE_SIZE: u32 = 224;
const IMAGE_MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const IMAGE_STD: [f32; 3] = [0.26862954, 0.261_302_6, 0.275_777_1];

/// The three files the model is built from, resolved once (downloading when
/// the cache is cold) so that a reload never touches the network.
#[derive(Clone, Debug)]
pub(crate) struct ModelPaths {
    pub(crate) vision: PathBuf,
    pub(crate) text: PathBuf,
    pub(crate) tokenizer: PathBuf,
}

/// One materialized CLIP model. Cloning is cheap (`Arc`), which is what lets
/// an in-flight encode keep the sessions alive after the engine dropped its
/// copy.
#[derive(Clone)]
pub(crate) struct LoadedModel {
    // ORT sessions need `&mut self` to run, so each tower gets its own lock.
    // The two towers are independent: a search never waits for an image batch.
    pub(crate) vision: Arc<Mutex<Session>>,
    pub(crate) text: Arc<Mutex<Session>>,
    pub(crate) tokenizer: Arc<Tokenizer>,
    // Drop guard, never read: MUST stay the last field, because struct fields
    // drop in declaration order, so the sessions above are freed before it
    // walks the arenas. It is shared by every clone, so the trim runs exactly
    // once — when the last clone (the logical unload point) drops.
    _trim: Arc<HeapTrim>,
}

/// Resolves the model files, downloading them when the cache is cold. This is
/// the only step that touches the network, so the reload path can use the
/// returned paths and an offline box keeps working once seeded.
pub(crate) fn resolve_model_paths(data_path: &str) -> Result<ModelPaths> {
    // NOTE: the `..` is intentional — `data_path/../data/models` keeps the
    // model cache in the repo-root ./data/models even when the app runs with a
    // sandboxed data dir (the E2E suite sets TURBO_PIX_DATA_PATH=test-e2e-data
    // while sharing the model cache; without the `..` the E2E server would
    // hang trying to download the model into the sandbox). Do not "simplify"
    // this path expression.
    let cache_dir = PathBuf::from(data_path).join("../data/models/clip");
    let local = ModelPaths {
        vision: cache_dir.join("vision.onnx"),
        text: cache_dir.join("text.onnx"),
        tokenizer: cache_dir.join("tokenizer.json"),
    };

    // Air-gapped/test fallback: the three files may be placed in the cache by
    // hand under stable names. The hashes are checked exactly like a download.
    if local.vision.is_file() && local.text.is_file() && local.tokenizer.is_file() {
        verify_sha256(&local.vision, VISION_SHA256)?;
        verify_sha256(&local.text, TEXT_SHA256)?;
        verify_sha256(&local.tokenizer, TOKENIZER_SHA256)?;
        return Ok(local);
    }

    let (owner, name) = REPO;
    let repo = hf_hub::HFClient::builder()
        .cache_dir(cache_dir)
        .build_sync()
        .context("Failed to build HuggingFace API for CLIP")?
        .model(owner, name);

    let vision = repo
        .download_file()
        .filename(VISION_FILE)
        .revision(REVISION)
        .send()
        .context("Failed to download the CLIP vision encoder")?;
    verify_sha256(&vision, VISION_SHA256)?;

    let text = repo
        .download_file()
        .filename(TEXT_FILE)
        .revision(REVISION)
        .send()
        .context("Failed to download the CLIP text encoder")?;
    verify_sha256(&text, TEXT_SHA256)?;

    let tokenizer = repo
        .download_file()
        .filename(TOKENIZER_FILE)
        .revision(REVISION)
        .send()
        .context("Failed to download the CLIP tokenizer")?;
    verify_sha256(&tokenizer, TOKENIZER_SHA256)?;

    Ok(ModelPaths {
        vision,
        text,
        tokenizer,
    })
}

/// Builds both sessions from already-resolved files: disk and parse only,
/// never the network, so a reload after an idle unload cannot stall on the hub.
pub(crate) fn load_from_paths(paths: &ModelPaths) -> Result<LoadedModel> {
    // The files were verified when resolved; verify again at the load
    // boundary so a tampered or truncated cache cannot reach the sessions on a
    // later idle reload.
    verify_sha256(&paths.vision, VISION_SHA256)?;
    verify_sha256(&paths.text, TEXT_SHA256)?;
    verify_sha256(&paths.tokenizer, TOKENIZER_SHA256)?;

    let started = Instant::now();
    let environment = shared_environment()?;
    let vision = build_session(
        &environment,
        &paths.vision,
        VISION_INPUT,
        VISION_OUTPUT,
        TensorElementType::Float32,
        &[3, IMAGE_SIZE as i64, IMAGE_SIZE as i64],
    )
    .context("Failed to load the CLIP vision encoder")?;
    let text = build_session(
        &environment,
        &paths.text,
        TEXT_INPUT,
        TEXT_OUTPUT,
        TensorElementType::Int64,
        &[CONTEXT_LENGTH as i64],
    )
    .context("Failed to load the CLIP text encoder")?;
    let tokenizer = Tokenizer::from_file(&paths.tokenizer)
        .map_err(anyhow::Error::msg)
        .context("Failed to load the CLIP tokenizer")?;

    log::info!(
        "CLIP ONNX model loaded in {:?} (RSS {} MiB)",
        started.elapsed(),
        rss_label(rss_mib())
    );
    Ok(LoadedModel {
        vision: Arc::new(Mutex::new(vision)),
        text: Arc::new(Mutex::new(text)),
        tokenizer: Arc::new(tokenizer),
        _trim: Arc::new(HeapTrim),
    })
}

/// Opens one ONNX graph and asserts the interface the encoders rely on.
///
/// The batch axis must be dynamic on both graphs: image encoding batches video
/// frames, and a fixed-batch export would reject every batch larger than one.
fn build_session(
    environment: &ort::environment::Environment,
    path: &Path,
    input_name: &str,
    output_name: &str,
    input_type: TensorElementType,
    input_tail: &[i64],
) -> Result<Session> {
    let session = Session::builder(environment)
        .map_err(|error| anyhow::anyhow!("ORT session builder: {error}"))?
        .with_intra_threads(model_threads())
        .map_err(|error| anyhow::anyhow!("ORT session threads: {error}"))?
        .commit_from_file(path)
        .map_err(|error| anyhow::anyhow!("ORT session load {}: {error}", path.display()))?;

    let inputs = session.inputs();
    if inputs.len() != 1 || inputs[0].name() != input_name {
        anyhow::bail!(
            "unexpected CLIP input for {}: expected one `{input_name}`, got {:?}",
            path.display(),
            inputs.iter().map(|input| input.name()).collect::<Vec<_>>()
        );
    }
    let input_shape = match inputs[0].dtype() {
        ValueType::Tensor { ty, shape, .. } if *ty == input_type => shape,
        other => anyhow::bail!(
            "unexpected CLIP input type for {}: expected {input_type:?}, got {other:?}",
            path.display()
        ),
    };
    assert_shape(path, input_shape, input_tail)?;

    let outputs = session.outputs();
    if outputs.len() != 1 || outputs[0].name() != output_name {
        anyhow::bail!(
            "unexpected CLIP output for {}: expected one `{output_name}`, got {:?}",
            path.display(),
            outputs
                .iter()
                .map(|output| output.name())
                .collect::<Vec<_>>()
        );
    }
    let (output_type, output_shape) = match outputs[0].dtype() {
        ValueType::Tensor { ty, shape, .. } => (ty, shape),
        other => anyhow::bail!(
            "unexpected CLIP output type for {}: expected a tensor, got {other:?}",
            path.display()
        ),
    };
    if *output_type != TensorElementType::Float32 {
        anyhow::bail!(
            "unexpected CLIP output type for {}: expected Float32, got {output_type:?}",
            path.display()
        );
    }
    assert_output_shape(path, output_shape)?;

    Ok(session)
}

fn assert_shape(path: &Path, shape: &Shape, input_tail: &[i64]) -> Result<()> {
    let dims: &[i64] = shape;
    // The pinned exports declare most axes as symbols (`batch_size`,
    // `sequence_length`, `height`, …), so a dimension is valid when it is
    // dynamic (-1) or matches the value this encoder feeds.
    let tail_matches = dims.len() == input_tail.len() + 1
        && dims[1..]
            .iter()
            .zip(input_tail)
            .all(|(dim, expected)| *dim == -1 || dim == expected);
    if dims.is_empty() || dims[0] != -1 || !tail_matches {
        anyhow::bail!(
            "unexpected CLIP input shape for {}: expected [-1, {:?}], got {dims:?}",
            path.display(),
            input_tail
        );
    }
    Ok(())
}

fn assert_output_shape(path: &Path, shape: &Shape) -> Result<()> {
    let dims: &[i64] = shape;
    let projection_ok = dims
        .get(1)
        .is_some_and(|dim| *dim == -1 || *dim == EMBEDDING_DIM as i64);
    if dims.len() != 2 || !matches!(dims[0], -1 | 1) || !projection_ok {
        anyhow::bail!(
            "unexpected CLIP output shape for {}: expected [batch, {EMBEDDING_DIM}], got {dims:?}",
            path.display()
        );
    }
    Ok(())
}

/// Encodes text into a normalized 512-dimensional vector.
pub(crate) fn encode_text(model: &LoadedModel, text: &str) -> Result<Vec<f32>> {
    let tokens = model
        .tokenizer
        .encode(text, true)
        .map_err(anyhow::Error::msg)?
        .get_ids()
        .to_vec();
    let tokens = pad_or_truncate_tokens(tokens);
    let ids: Vec<i64> = tokens.into_iter().map(i64::from).collect();

    let tensor = Tensor::<i64>::from_array(([1usize, CONTEXT_LENGTH], ids))
        .map_err(|error| anyhow::anyhow!("CLIP text input tensor: {error}"))?;
    let mut session = lock_session(&model.text);
    let outputs = session
        .run(ort::inputs![TEXT_INPUT => tensor])
        .map_err(|error| anyhow::anyhow!("CLIP text run: {error}"))?;

    extract_embeddings(&outputs, TEXT_OUTPUT, 1)?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("CLIP text encoder returned no embedding"))
}

/// Encodes an image into a normalized 512-dimensional vector.
pub(crate) fn encode_image(model: &LoadedModel, image_path: &Path) -> Result<Vec<f32>> {
    let mut vectors = encode_image_batch(model, &[image_path.to_path_buf()])?;
    vectors
        .pop()
        .ok_or_else(|| anyhow::anyhow!("CLIP vision encoder returned no embedding"))
}

/// Encodes a batch of images into normalized 512-dimensional vectors.
pub(crate) fn encode_image_batch(
    model: &LoadedModel,
    image_paths: &[PathBuf],
) -> Result<Vec<Vec<f32>>> {
    use rayon::prelude::*;

    let planes: Vec<Vec<f32>> = image_paths
        .par_iter()
        .map(|path| preprocess_image(path))
        .collect::<Result<_>>()?;

    if planes.is_empty() {
        return Err(anyhow::anyhow!("Cannot encode empty batch"));
    }
    let batch = planes.len();
    let mut pixels = Vec::with_capacity(batch * 3 * IMAGE_SIZE as usize * IMAGE_SIZE as usize);
    for plane in planes {
        pixels.extend_from_slice(&plane);
    }

    let tensor =
        Tensor::<f32>::from_array(([batch, 3, IMAGE_SIZE as usize, IMAGE_SIZE as usize], pixels))
            .map_err(|error| anyhow::anyhow!("CLIP vision input tensor: {error}"))?;
    let mut session = lock_session(&model.vision);
    let outputs = session
        .run(ort::inputs![VISION_INPUT => tensor])
        .map_err(|error| anyhow::anyhow!("CLIP vision run: {error}"))?;

    extract_embeddings(&outputs, VISION_OUTPUT, batch)
}

/// Decodes, resizes and normalizes one image into an NCHW f32 plane.
fn preprocess_image(path: &Path) -> Result<Vec<f32>> {
    let img = if raw_processor::is_raw_file(path) {
        raw_processor::decode_raw_to_dynamic_image(path)
            .context("Failed to decode RAW file for CLIP encoding")?
    } else {
        image::open(path)?
    };

    let img = if img.width() == IMAGE_SIZE && img.height() == IMAGE_SIZE {
        img
    } else {
        img.resize_exact(
            IMAGE_SIZE,
            IMAGE_SIZE,
            image::imageops::FilterType::Triangle,
        )
    };
    let img = img.to_rgb8();

    let plane = IMAGE_SIZE as usize * IMAGE_SIZE as usize;
    let mut data = vec![0.0_f32; 3 * plane];
    for (x, y, pixel) in img.enumerate_pixels() {
        let index = y as usize * IMAGE_SIZE as usize + x as usize;
        for channel in 0..3 {
            let value = pixel[channel] as f32 / 255.0;
            data[channel * plane + index] = (value - IMAGE_MEAN[channel]) / IMAGE_STD[channel];
        }
    }
    Ok(data)
}

/// Reads `[batch, 512]` from the named output and L2-normalizes every row.
fn extract_embeddings(
    outputs: &SessionOutputs<'_>,
    name: &str,
    expected_batch: usize,
) -> Result<Vec<Vec<f32>>> {
    let value = outputs
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("CLIP output `{name}` is missing"))?;
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|error| anyhow::anyhow!("CLIP output `{name}`: {error}"))?;
    let dims: &[i64] = shape;
    if dims != [expected_batch as i64, EMBEDDING_DIM as i64]
        || data.len() != expected_batch * EMBEDDING_DIM
    {
        anyhow::bail!(
            "unexpected CLIP output `{name}` shape: expected [{expected_batch}, {EMBEDDING_DIM}], got {dims:?}"
        );
    }

    let (rows, remainder) = data.as_chunks::<EMBEDDING_DIM>();
    debug_assert!(
        remainder.is_empty(),
        "embedding output is not a multiple of {EMBEDDING_DIM}"
    );
    rows.iter()
        .map(|row| normalize_vector(row.to_vec()))
        .collect()
}

/// Computes L2 normalization of a semantic vector.
pub(crate) fn normalize_vector(mut vector: Vec<f32>) -> Result<Vec<f32>> {
    if vector.len() != EMBEDDING_DIM {
        anyhow::bail!(
            "Unexpected embedding length: expected {EMBEDDING_DIM}, got {}",
            vector.len()
        );
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        anyhow::bail!("Cannot normalize a zero-length semantic vector");
    }
    for value in &mut vector {
        *value /= norm;
    }
    Ok(vector)
}

/// Averages per-frame embeddings into one video embedding (not normalized).
pub(crate) fn average_vectors(vectors: &[Vec<f32>]) -> Result<Vec<f32>> {
    let Some(first) = vectors.first() else {
        return Err(anyhow::anyhow!("Cannot average an empty batch"));
    };
    if first.len() != EMBEDDING_DIM {
        anyhow::bail!(
            "Unexpected embedding length: expected {EMBEDDING_DIM}, got {}",
            first.len()
        );
    }

    let mut mean = vec![0.0_f32; EMBEDDING_DIM];
    for vector in vectors {
        if vector.len() != EMBEDDING_DIM {
            anyhow::bail!(
                "Unexpected embedding length: expected {EMBEDDING_DIM}, got {}",
                vector.len()
            );
        }
        for (sum, value) in mean.iter_mut().zip(vector) {
            *sum += value;
        }
    }
    let count = vectors.len() as f32;
    for value in &mut mean {
        *value /= count;
    }
    Ok(mean)
}

/// Ensures exactly `CONTEXT_LENGTH` tokens: pads short inputs with EOT and
/// truncates long inputs to `CONTEXT_LENGTH - 1` tokens plus a re-appended
/// EOT (the tokenizer's own trailing EOT would otherwise be dropped by a
/// plain truncate).
fn pad_or_truncate_tokens(mut tokens: Vec<u32>) -> Vec<u32> {
    if tokens.len() < CONTEXT_LENGTH {
        tokens.resize(CONTEXT_LENGTH, EOT_TOKEN);
    } else {
        // The tokenizer already appended EOT, so truncating to
        // CONTEXT_LENGTH would drop it; keep CONTEXT_LENGTH - 1 tokens and
        // re-append EOT as the final token.
        tokens.truncate(CONTEXT_LENGTH - 1);
        tokens.push(EOT_TOKEN);
    }
    tokens
}

/// ORT sessions may only run with `&mut self`; a poisoned lock is recoverable
/// because a panic in an inference call cannot leave the graph half-mutated.
fn lock_session(session: &Mutex<Session>) -> MutexGuard<'_, Session> {
    session
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Loads the model once per test binary for the modules that need one.
///
/// `OnceLock`, not `LazyLock`: the initializer downloads the weights when the
/// cache is cold, so a failed attempt must be retried by the next test instead
/// of poisoning every test that touches the cell.
#[cfg(test)]
pub(crate) fn cached_test_model() -> LoadedModel {
    use std::sync::OnceLock;

    static MODEL_CACHE: OnceLock<LoadedModel> = OnceLock::new();
    MODEL_CACHE
        .get_or_init(|| {
            // Relative to the project root, where `cargo test` runs.
            let paths = resolve_model_paths("./data").expect("Failed to resolve model files");
            load_from_paths(&paths).expect("Failed to load model")
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_short_token_sequences_with_eot() {
        let tokens = pad_or_truncate_tokens(vec![1, 2, 3]);
        assert_eq!(tokens.len(), CONTEXT_LENGTH);
        assert_eq!(tokens[..3], [1, 2, 3]);
        assert_eq!(tokens[3], EOT_TOKEN);
        assert_eq!(tokens.last(), Some(&EOT_TOKEN));
    }

    #[test]
    fn truncates_long_token_sequences_and_reappends_eot() {
        let long: Vec<u32> = (0..200).map(|i| i as u32).collect();
        let tokens = pad_or_truncate_tokens(long);
        assert_eq!(tokens.len(), CONTEXT_LENGTH);
        assert_eq!(
            tokens[..CONTEXT_LENGTH - 1],
            (0..76).map(|i| i as u32).collect::<Vec<u32>>()
        );
        assert_eq!(tokens[CONTEXT_LENGTH - 1], EOT_TOKEN);
    }

    #[test]
    fn leaves_exact_length_sequences_alone() {
        let exact: Vec<u32> = (0..CONTEXT_LENGTH).map(|i| i as u32).collect();
        let tokens = pad_or_truncate_tokens(exact.clone());
        // CONTEXT_LENGTH input: keep first CONTEXT_LENGTH - 1, re-append EOT.
        assert_eq!(tokens.len(), CONTEXT_LENGTH);
        assert_eq!(tokens[..CONTEXT_LENGTH - 1], exact[..CONTEXT_LENGTH - 1]);
        assert_eq!(tokens[CONTEXT_LENGTH - 1], EOT_TOKEN);
    }

    #[test]
    fn normalizes_to_unit_length() {
        let mut vector = vec![0.0_f32; EMBEDDING_DIM];
        vector[0] = 3.0;
        vector[1] = 4.0;
        let normalized = normalize_vector(vector).unwrap();
        let norm = normalized.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
        assert!((normalized[0] - 0.6).abs() < 1e-5);
        assert!((normalized[1] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn rejects_zero_vector_normalization() {
        assert!(normalize_vector(vec![0.0; EMBEDDING_DIM]).is_err());
    }

    #[test]
    fn preprocesses_to_normalized_chw_planes() {
        let mut img = image::RgbImage::from_pixel(IMAGE_SIZE, IMAGE_SIZE, image::Rgb([0, 0, 0]));
        img.put_pixel(10, 5, image::Rgb([255, 128, 32]));
        let file = tempfile::Builder::new()
            .suffix(".png")
            .tempfile()
            .expect("create temp png");
        img.save(file.path()).expect("write temp png");

        let data = preprocess_image(file.path()).expect("preprocess image");

        let plane = IMAGE_SIZE as usize * IMAGE_SIZE as usize;
        assert_eq!(data.len(), 3 * plane);

        // HWC `(10, 5)` lands in every CHW channel plane at `5 * 224 + 10`.
        let index = 5 * IMAGE_SIZE as usize + 10;
        for (channel, &byte) in [255_u8, 128, 32].iter().enumerate() {
            let value = f32::from(byte) / 255.0;
            let expected = (value - IMAGE_MEAN[channel]) / IMAGE_STD[channel];
            assert_eq!(data[channel * plane + index], expected);

            // Pixel (0, 0) stays black, i.e. normalized zero.
            let black = -IMAGE_MEAN[channel] / IMAGE_STD[channel];
            assert_eq!(data[channel * plane], black);
        }
    }

    #[test]
    fn accepts_dynamic_and_matching_input_shapes() {
        let path = Path::new("vision.onnx");
        for dims in [vec![-1_i64, 3, 224, 224], vec![-1, -1, -1, -1]] {
            assert!(assert_shape(path, &Shape::from(dims), &[3, 224, 224]).is_ok());
        }
    }

    #[test]
    fn rejects_wrong_input_shapes() {
        let path = Path::new("vision.onnx");
        for dims in [
            vec![1, 3, 224, 224],  // batch axis must be dynamic
            vec![-1, 3, 224, 223], // height mismatch
            vec![-1_i64, 3, 224],  // too few axes
            vec![],                // empty shape
        ] {
            assert!(assert_shape(path, &Shape::from(dims), &[3, 224, 224]).is_err());
        }
    }

    #[test]
    fn accepts_matching_output_shapes() {
        let path = Path::new("vision.onnx");
        for dims in [
            vec![-1, EMBEDDING_DIM as i64],
            vec![1, EMBEDDING_DIM as i64],
            vec![-1, -1],
            vec![1, -1],
        ] {
            assert!(assert_output_shape(path, &Shape::from(dims)).is_ok());
        }
    }

    #[test]
    fn rejects_wrong_output_shapes() {
        let path = Path::new("vision.onnx");
        for dims in [
            vec![EMBEDDING_DIM as i64],        // missing projection axis
            vec![2, EMBEDDING_DIM as i64],     // batch must be dynamic or 1
            vec![1, EMBEDDING_DIM as i64 - 1], // wrong projection width
            vec![1, EMBEDDING_DIM as i64, 1],  // too many axes
        ] {
            assert!(assert_output_shape(path, &Shape::from(dims)).is_err());
        }
    }

    #[test]
    fn averages_per_frame_embeddings() {
        let mut first = vec![0.0_f32; EMBEDDING_DIM];
        first[0] = 2.0;
        let mut second = vec![0.0_f32; EMBEDDING_DIM];
        second[0] = 4.0;
        let mean = average_vectors(&[first, second]).unwrap();
        assert!((mean[0] - 3.0).abs() < 1e-6);
    }
}
