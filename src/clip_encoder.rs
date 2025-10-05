use image::DynamicImage;
use instant_clip_tokenizer::Tokenizer;
use ndarray::{Array, Array4};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Value;
use std::path::Path;
use std::sync::RwLock;

/// CLIP encoder for generating image and text embeddings
/// Supports multilingual text encoding (100+ languages including German, English)
/// Uses RwLock for interior mutability to allow concurrent inference
pub struct ClipEncoder {
    visual_session: RwLock<Session>,
    textual_session: RwLock<Session>,
    tokenizer: Tokenizer,
}

impl ClipEncoder {
    /// Create a new CLIP encoder from model files
    ///
    /// # Arguments
    /// * `model_path` - Directory containing visual.onnx, textual.onnx, and tokenizer.json
    pub fn new(model_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        log::info!("Loading CLIP models from {:?}", model_path);

        // Optimize thread count based on CPU cores
        let num_threads = num_cpus::get();
        log::info!("Using {} threads per ONNX session", num_threads);

        // Load visual encoder (for images)
        let visual_path = model_path.join("visual.onnx");
        let visual_session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(num_threads)?
            .with_inter_threads(2)?
            .commit_from_file(visual_path)?;

        log::info!("Visual encoder loaded");

        // Load textual encoder (for text queries)
        let textual_path = model_path.join("textual.onnx");
        let textual_session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(num_threads)?
            .with_inter_threads(2)?
            .commit_from_file(textual_path)?;

        log::info!("Textual encoder loaded");

        // Load tokenizer
        let tokenizer = Tokenizer::new();

        log::info!("CLIP encoder initialized successfully");

        Ok(Self {
            visual_session: RwLock::new(visual_session),
            textual_session: RwLock::new(textual_session),
            tokenizer,
        })
    }

    /// Generate embedding vector for an image (async with I/O optimization)
    ///
    /// # Arguments
    /// * `image_path` - Path to the image file
    ///
    /// # Returns
    /// A normalized 768-dimensional embedding vector
    #[allow(dead_code)]
    pub async fn encode_image_async(&self, image_path: &Path) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        // Load image asynchronously to reduce I/O blocking
        let path = image_path.to_path_buf();
        let bytes = tokio::fs::read(&path).await?;

        // Decode image in blocking task pool (CPU intensive)
        let img = tokio::task::spawn_blocking(move || {
            image::load_from_memory(&bytes)
        }).await??;

        // Preprocess for CLIP
        let preprocessed = self.preprocess_image(img)?;

        // Convert ndarray to ort::Value
        let input_value = Value::from_array(preprocessed)?;

        // Run inference through visual encoder
        // Use RwLock for thread-safe interior mutability (Session.run() requires &mut self)
        let mut session = self.visual_session.write().unwrap();
        let outputs = session.run(ort::inputs!["image" => input_value])?;

        // Extract embedding from output (use first output by index)
        let embedding = outputs[0]
            .try_extract_array::<f32>()?
            .view()
            .to_owned();

        // Normalize to unit length for cosine similarity
        let embedding_slice = embedding
            .as_slice()
            .ok_or("Failed to convert embedding to slice")?;
        let normalized = Self::normalize_vector(embedding_slice);

        Ok(normalized)
    }

    /// Generate embedding vector for an image (synchronous version for compatibility)
    ///
    /// # Arguments
    /// * `image_path` - Path to the image file
    ///
    /// # Returns
    /// A normalized 768-dimensional embedding vector
    #[allow(dead_code)]
    pub fn encode_image(&self, image_path: &Path) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        // Load image
        let img = image::open(image_path)?;

        // Preprocess for CLIP
        let preprocessed = self.preprocess_image(img)?;

        // Convert ndarray to ort::Value
        let input_value = Value::from_array(preprocessed)?;

        // Run inference through visual encoder
        // Use RwLock for thread-safe interior mutability (Session.run() requires &mut self)
        let mut session = self.visual_session.write().unwrap();
        let outputs = session.run(ort::inputs!["image" => input_value])?;

        // Extract embedding from output (use first output by index)
        let embedding = outputs[0]
            .try_extract_array::<f32>()?
            .view()
            .to_owned();

        // Normalize to unit length for cosine similarity
        let embedding_slice = embedding
            .as_slice()
            .ok_or("Failed to convert embedding to slice")?;
        let normalized = Self::normalize_vector(embedding_slice);

        Ok(normalized)
    }

    /// Generate embedding vector for text query (multilingual)
    ///
    /// # Arguments
    /// * `text` - Search query in any supported language (e.g., "cat", "Katze", "gato")
    ///
    /// # Returns
    /// A normalized 768-dimensional embedding vector
    pub fn encode_text(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        // Tokenize text (handles multilingual input automatically)
        let mut tokens = Vec::new();
        self.tokenizer.encode(text, &mut tokens);

        // Convert Token to i32 (ONNX model expects int32, not int64)
        let mut token_ids: Vec<i32> = tokens.iter().map(|t| t.to_u16() as i32).collect();

        // CLIP expects exactly 77 tokens - pad or truncate
        const CLIP_CONTEXT_LENGTH: usize = 77;
        token_ids.resize(CLIP_CONTEXT_LENGTH, 0); // Pad with zeros

        // Convert to ndarray format expected by ONNX model
        let input_ids = Array::from_shape_vec((1, CLIP_CONTEXT_LENGTH), token_ids)?;

        // Convert to ort::Value
        let input_value = Value::from_array(input_ids)?;

        // Run inference through textual encoder
        // Use RwLock for thread-safe interior mutability (Session.run() requires &mut self)
        let mut session = self.textual_session.write().unwrap();
        let outputs = session.run(ort::inputs!["text" => input_value])?;

        // Extract embedding from output (use first output by index)
        let embedding = outputs[0]
            .try_extract_array::<f32>()?
            .view()
            .to_owned();

        // Normalize to unit length for cosine similarity
        let embedding_slice = embedding
            .as_slice()
            .ok_or("Failed to convert embedding to slice")?;
        let normalized = Self::normalize_vector(embedding_slice);

        Ok(normalized)
    }

    /// Preprocess image for CLIP model
    /// SigLIP CLIP expects 384x384 RGB images with ImageNet normalization
    fn preprocess_image(
        &self,
        img: DynamicImage,
    ) -> Result<Array4<f32>, Box<dyn std::error::Error>> {
        // Resize to 384x384 (SigLIP input size, larger than standard CLIP's 224)
        // Triangle filter is faster than Lanczos3 with minimal quality loss for ML preprocessing
        const SIZE: u32 = 384;
        let img = img.resize_exact(SIZE, SIZE, image::imageops::FilterType::Triangle);
        let rgb = img.to_rgb8();

        // ImageNet normalization parameters (same as PyTorch/Python CLIP)
        let mean = [0.48145466, 0.4578275, 0.40821073];
        let std = [0.26862954, 0.26130258, 0.27577711];

        // Convert to ndarray with shape [1, 3, 384, 384] (batch, channels, height, width)
        // Vectorized initialization is faster than nested loops
        let array = Array4::<f32>::from_shape_fn((1, 3, SIZE as usize, SIZE as usize), |(_, c, y, x)| {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            // Normalize: (pixel/255 - mean) / std
            (pixel[c] as f32 / 255.0 - mean[c]) / std[c]
        });

        Ok(array)
    }

    /// Normalize vector to unit length (L2 normalization)
    /// This ensures cosine similarity can be computed as simple dot product
    fn normalize_vector(vec: &[f32]) -> Vec<f32> {
        let magnitude = vec.iter().map(|x| x * x).sum::<f32>().sqrt();

        // Avoid division by zero
        if magnitude == 0.0 {
            return vec.to_vec();
        }

        vec.iter().map(|x| x / magnitude).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_vector() {
        let vec = vec![3.0, 4.0];
        let normalized = ClipEncoder::normalize_vector(&vec);

        // Length should be 1.0
        let length = normalized.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 0.0001);

        // Values should be [0.6, 0.8]
        assert!((normalized[0] - 0.6).abs() < 0.0001);
        assert!((normalized[1] - 0.8).abs() < 0.0001);
    }

    #[test]
    fn test_normalize_zero_vector() {
        let vec = vec![0.0, 0.0];
        let normalized = ClipEncoder::normalize_vector(&vec);

        // Should return original vector
        assert_eq!(normalized, vec);
    }
}
