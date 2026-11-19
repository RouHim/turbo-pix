//! ONNX inference: SCRFD detection, ArcFace alignment/embedding and the face
//! crop used by the People view.

use std::sync::Mutex;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{Rgb, RgbImage};
use ort::session::Session;

use super::detector::decode_scrfd;
use super::geometry::{
    apply_affine, l2_normalize, letterbox, map_landmarks_to_source, map_rect_to_source, nms,
    similarity_transform, Detection, Point, Rect,
};
use super::model::{load_models, resolve_model_paths, FaceModels};

pub const DETECT_INPUT: u32 = 640;
pub const DETECT_SCORE_THRESHOLD: f32 = 0.5;
pub const DETECT_NMS_IOU: f32 = 0.4;
pub const MIN_FACE_PIXELS: f32 = 20.0;
pub const FACE_SIZE: u32 = 112;
pub const CROP_SIZE: u32 = 200;

/// The ArcFace 112×112 five-point template (`src` in the alignment solve).
const ARCFACE_TEMPLATE: [Point; 5] = [
    Point::new(38.2946, 51.6963),
    Point::new(73.5318, 51.5014),
    Point::new(56.0252, 71.7366),
    Point::new(41.5493, 92.3655),
    Point::new(70.7299, 92.2041),
];

#[derive(Clone, Debug)]
pub struct DetectedFace {
    /// Normalised (0..1) relative to the analysed thumbnail.
    pub bbox: Rect,
    pub quality: f32,
    pub embedding: Vec<f32>,
    /// JPEG bytes for the People card, already resized to [`CROP_SIZE`].
    pub crop: Vec<u8>,
}

pub struct FaceEngine {
    data_path: String,
    models: Mutex<Option<FaceModels>>,
}

impl FaceEngine {
    pub fn new(data_path: &str) -> Self {
        Self {
            data_path: data_path.to_string(),
            models: Mutex::new(None),
        }
    }

    /// Resolves/downloads and loads both sessions once. Kept public so the
    /// pipeline can fail the phase once instead of recording a model error for
    /// every pending photo.
    pub fn ensure_loaded(&self) -> Result<()> {
        let mut guard = self
            .models
            .lock()
            .map_err(|_| anyhow::anyhow!("face model lock poisoned"))?;
        if guard.is_none() {
            let paths = resolve_model_paths(&self.data_path)?;
            let threads = num_cpus::get().clamp(1, 4);
            *guard = Some(load_models(&paths, threads)?);
            log::info!("Face models loaded (ORT threads: {threads})");
        }
        Ok(())
    }

    /// Processes one image (normally the 800 px thumbnail) and returns every
    /// detected face with its normalised embedding and crop.
    pub fn process_image(&self, image: &RgbImage) -> Result<Vec<DetectedFace>> {
        let detections = self.detect(image)?;
        let mut faces = Vec::with_capacity(detections.len());

        for detection in detections {
            let bbox = detection
                .rect
                .clamp(image.width() as f32, image.height() as f32);
            if bbox.w < MIN_FACE_PIXELS || bbox.h < MIN_FACE_PIXELS {
                continue;
            }

            let Some(embedding) = self.embed(image, &detection.landmarks)? else {
                continue;
            };
            let crop = crop_face(image, bbox)?;

            faces.push(DetectedFace {
                bbox: Rect::new(
                    bbox.x / image.width() as f32,
                    bbox.y / image.height() as f32,
                    bbox.w / image.width() as f32,
                    bbox.h / image.height() as f32,
                ),
                quality: detection.score,
                embedding,
                crop,
            });
        }

        Ok(faces)
    }

    fn detect(&self, image: &RgbImage) -> Result<Vec<Detection>> {
        self.ensure_loaded()?;
        let mut guard = self
            .models
            .lock()
            .map_err(|_| anyhow::anyhow!("face model lock poisoned"))?;
        let models = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("face models unavailable"))?;

        detect_with_session(&mut models.detector, image)
    }

    fn embed(&self, image: &RgbImage, landmarks: &[Point; 5]) -> Result<Option<Vec<f32>>> {
        let Some(matrix) = similarity_transform(landmarks, &ARCFACE_TEMPLATE) else {
            return Ok(None);
        };
        let determinant = matrix[0] * matrix[4] - matrix[1] * matrix[3];
        if !determinant.is_finite() || determinant.abs() < 1e-6 {
            return Ok(None);
        }

        let face = warp_affine(image, &matrix, FACE_SIZE, FACE_SIZE);
        let mut input = vec![0.0_f32; 3 * FACE_SIZE as usize * FACE_SIZE as usize];
        for (x, y, pixel) in face.enumerate_pixels() {
            let base = y as usize * FACE_SIZE as usize + x as usize;
            for channel in 0..3 {
                input[channel * FACE_SIZE as usize * FACE_SIZE as usize + base] =
                    (pixel[channel] as f32 - 127.5) / 127.5;
            }
        }

        let mut guard = self
            .models
            .lock()
            .map_err(|_| anyhow::anyhow!("face model lock poisoned"))?;
        let models = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("face models unavailable"))?;
        let tensor = ort::value::Tensor::<f32>::from_array((
            [1usize, 3, FACE_SIZE as usize, FACE_SIZE as usize],
            input,
        ))
        .map_err(|error| anyhow::anyhow!("ArcFace input tensor: {error}"))?;
        let outputs = models
            .recognizer
            .run(ort::inputs!["input.1" => tensor])
            .map_err(|error| anyhow::anyhow!("ArcFace run: {error}"))?;

        let Some((_, value)) = outputs.iter().next() else {
            return Ok(None);
        };
        let (_, data) = value
            .try_extract_tensor::<f32>()
            .map_err(|error| anyhow::anyhow!("ArcFace output: {error}"))?;
        let mut embedding = data.to_vec();
        if embedding.len() != 512 || !l2_normalize(&mut embedding) {
            return Ok(None);
        }
        Ok(Some(embedding))
    }
}

fn detect_with_session(session: &mut Session, image: &RgbImage) -> Result<Vec<Detection>> {
    let transform = letterbox(image.width(), image.height(), DETECT_INPUT);
    let resized = image::imageops::resize(
        image,
        transform.resized_w,
        transform.resized_h,
        FilterType::Triangle,
    );

    let target = DETECT_INPUT as usize;
    let mut input = vec![0.0_f32; 3 * target * target];
    let pad_x = transform.pad_x.round() as usize;
    let pad_y = transform.pad_y.round() as usize;
    for (x, y, pixel) in resized.enumerate_pixels() {
        let px = pad_x + x as usize;
        let py = pad_y + y as usize;
        if px >= target || py >= target {
            continue;
        }
        let base = py * target + px;
        for channel in 0..3 {
            input[channel * target * target + base] = (pixel[channel] as f32 - 127.5) / 128.0;
        }
    }

    let tensor = ort::value::Tensor::<f32>::from_array(([1usize, 3, target, target], input))
        .map_err(|error| anyhow::anyhow!("SCRFD input tensor: {error}"))?;
    let outputs = session
        .run(ort::inputs!["input.1" => tensor])
        .map_err(|error| anyhow::anyhow!("SCRFD run: {error}"))?;

    let mut arrays: Vec<Vec<f32>> = Vec::with_capacity(9);
    for (_, value) in outputs.iter() {
        let (_, data) = value
            .try_extract_tensor::<f32>()
            .map_err(|error| anyhow::anyhow!("SCRFD output: {error}"))?;
        arrays.push(data.to_vec());
    }
    if arrays.len() != 9 {
        anyhow::bail!("SCRFD returned {} outputs, expected 9", arrays.len());
    }

    let decoded = decode_scrfd(
        &[&arrays[0], &arrays[1], &arrays[2]],
        &[&arrays[3], &arrays[4], &arrays[5]],
        &[&arrays[6], &arrays[7], &arrays[8]],
        target,
        target,
        DETECT_SCORE_THRESHOLD,
    );

    let mapped = decoded
        .into_iter()
        .map(|detection| Detection {
            rect: map_rect_to_source(detection.rect, &transform),
            score: detection.score,
            landmarks: map_landmarks_to_source(&detection.landmarks, &transform),
        })
        .collect();

    Ok(nms(mapped, DETECT_NMS_IOU))
}

fn crop_face(image: &RgbImage, bbox: Rect) -> Result<Vec<u8>> {
    let side = bbox.w.max(bbox.h) * 1.35;
    let center_x = bbox.x + bbox.w / 2.0;
    let center_y = bbox.y + bbox.h / 2.0;
    let crop = Rect::new(center_x - side / 2.0, center_y - side / 2.0, side, side)
        .clamp(image.width() as f32, image.height() as f32);

    let x = crop.x.floor().max(0.0) as u32;
    let y = crop.y.floor().max(0.0) as u32;
    let width = crop.w.ceil() as u32;
    let height = crop.h.ceil() as u32;
    let width = width.min(image.width().saturating_sub(x)).max(1);
    let height = height.min(image.height().saturating_sub(y)).max(1);

    let face = image::imageops::crop_imm(image, x, y, width, height).to_image();
    let face = image::imageops::resize(&face, CROP_SIZE, CROP_SIZE, FilterType::Triangle);

    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90);
    encoder
        .encode_image(&face)
        .context("Failed to encode face crop")?;
    Ok(bytes)
}

/// Samples `source` through the inverse of the alignment matrix. Bilinear
/// sampling keeps the 112×112 ArcFace input smooth even when the source face is
/// smaller than the template.
fn warp_affine(source: &RgbImage, matrix: &[f32; 6], width: u32, height: u32) -> RgbImage {
    let determinant = matrix[0] * matrix[4] - matrix[1] * matrix[3];
    let inverse = [
        matrix[4] / determinant,
        -matrix[1] / determinant,
        (matrix[1] * matrix[5] - matrix[4] * matrix[2]) / determinant,
        -matrix[3] / determinant,
        matrix[0] / determinant,
        (matrix[3] * matrix[2] - matrix[0] * matrix[5]) / determinant,
    ];

    let mut output = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let point = apply_affine(&inverse, Point::new(x as f32, y as f32));
            output.put_pixel(x, y, sample_bilinear(source, point));
        }
    }
    output
}

fn sample_bilinear(image: &RgbImage, point: Point) -> Rgb<u8> {
    let max_x = image.width().saturating_sub(1) as f32;
    let max_y = image.height().saturating_sub(1) as f32;
    let x = point.x.clamp(0.0, max_x);
    let y = point.y.clamp(0.0, max_y);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width().saturating_sub(1));
    let y1 = (y0 + 1).min(image.height().saturating_sub(1));
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;

    let p00 = image.get_pixel(x0, y0);
    let p10 = image.get_pixel(x1, y0);
    let p01 = image.get_pixel(x0, y1);
    let p11 = image.get_pixel(x1, y1);

    let mut channel = [0_u8; 3];
    for index in 0..3 {
        let top = p00[index] as f32 * (1.0 - fx) + p10[index] as f32 * fx;
        let bottom = p01[index] as f32 * (1.0 - fx) + p11[index] as f32 * fx;
        channel[index] = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
    }
    Rgb(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_is_clamped_to_the_source_image() {
        let image = RgbImage::from_pixel(100, 100, Rgb([10, 20, 30]));
        let bytes = crop_face(&image, Rect::new(90.0, 90.0, 40.0, 40.0)).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn warp_identity_preserves_a_pixel_grid() {
        let mut image = RgbImage::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                image.put_pixel(x, y, Rgb([x as u8 * 10, y as u8 * 10, 7]));
            }
        }
        let warped = warp_affine(&image, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 8, 8);
        assert_eq!(warped, image);
    }
}
