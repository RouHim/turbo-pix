//! Geometry primitives shared by the detector, the aligner and the clustering
//! step. Everything is pure so the coordinate conventions can be tested without
//! loading a model.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn area(self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    pub fn clamp(self, width: f32, height: f32) -> Self {
        let x1 = self.x.clamp(0.0, width);
        let y1 = self.y.clamp(0.0, height);
        let x2 = self.right().clamp(0.0, width);
        let y2 = self.bottom().clamp(0.0, height);
        Self::new(x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0))
    }
}

#[derive(Clone, Debug)]
pub struct Detection {
    pub rect: Rect,
    pub score: f32,
    pub landmarks: [Point; 5],
}

/// The resize/pad transform that places a source image into the detector's
/// square input. Boxes stay in input coordinates until they are mapped back
/// through this struct.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    pub scale: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub resized_w: u32,
    pub resized_h: u32,
}

/// Fits `width`×`height` into a `target`×`target` square without distortion.
pub fn letterbox(width: u32, height: u32, target: u32) -> Letterbox {
    let target = target as f32;
    let scale = (target / width.max(1) as f32).min(target / height.max(1) as f32);
    let resized_w = ((width as f32 * scale).round() as u32).clamp(1, target as u32);
    let resized_h = ((height as f32 * scale).round() as u32).clamp(1, target as u32);
    Letterbox {
        scale,
        pad_x: (target - resized_w as f32) / 2.0,
        pad_y: (target - resized_h as f32) / 2.0,
        resized_w,
        resized_h,
    }
}

/// Maps a rectangle from detector-input coordinates back to source pixels.
pub fn map_rect_to_source(rect: Rect, transform: &Letterbox) -> Rect {
    let x = (rect.x - transform.pad_x) / transform.scale;
    let y = (rect.y - transform.pad_y) / transform.scale;
    Rect::new(x, y, rect.w / transform.scale, rect.h / transform.scale)
}

/// Maps the five landmark points from detector-input coordinates back to source
/// pixels.
pub fn map_landmarks_to_source(points: &[Point; 5], transform: &Letterbox) -> [Point; 5] {
    points.map(|point| {
        Point::new(
            (point.x - transform.pad_x) / transform.scale,
            (point.y - transform.pad_y) / transform.scale,
        )
    })
}

/// Intersection-over-union, clamped to `0..=1`.
pub fn iou(a: Rect, b: Rect) -> f32 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = a.right().min(b.right());
    let y2 = a.bottom().min(b.bottom());
    let intersection = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a.area() + b.area() - intersection;
    if union <= f32::EPSILON {
        0.0
    } else {
        (intersection / union).clamp(0.0, 1.0)
    }
}

/// Greedy non-maximum suppression, highest score first.
pub fn nms(mut detections: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
    detections.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<Detection> = Vec::with_capacity(detections.len());
    for detection in detections {
        if kept
            .iter()
            .all(|other| iou(detection.rect, other.rect) <= iou_threshold)
        {
            kept.push(detection);
        }
    }
    kept
}

/// Cosine distance (`1 - cosine similarity`) for L2-normalised vectors.
/// A zero-length vector is maximally distant.
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (left, right) in a.iter().zip(b.iter()) {
        dot += left * right;
        norm_a += left * left;
        norm_b += right * right;
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        return 1.0;
    }
    (1.0 - dot / (norm_a.sqrt() * norm_b.sqrt())).clamp(0.0, 2.0)
}

/// L2-normalises in place. Returns false for a zero vector, which is not a
/// usable embedding.
pub fn l2_normalize(vector: &mut [f32]) -> bool {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON || !norm.is_finite() {
        return false;
    }
    vector.iter_mut().for_each(|value| *value /= norm);
    true
}

/// The 2×3 affine matrix (`[a, b, tx, c, d, ty]`) that best maps `src` onto
/// `dst` under a similarity transform (rotation, uniform scale, translation).
pub fn similarity_transform(src: &[Point; 5], dst: &[Point; 5]) -> Option<[f32; 6]> {
    // x' = a*x + b*y + tx
    // y' = -b*x + a*y + ty
    // Four unknowns, two equations per correspondence. Normal equations keep
    // the solve small and deterministic; five points make it overdetermined.
    let mut normal = [[0.0_f64; 4]; 4];
    let mut rhs = [0.0_f64; 4];
    for (source, target) in src.iter().zip(dst.iter()) {
        let x = source.x as f64;
        let y = source.y as f64;
        let tx = target.x as f64;
        let ty = target.y as f64;
        // [a, b, tx, ty]:
        //   x' = a*x + b*y + tx
        //   y' = -b*x + a*y + ty  ->  coefficients [y, -x, 0, 1]
        let rows = [([x, y, 1.0, 0.0], tx), ([y, -x, 0.0, 1.0], ty)];
        for (row, value) in rows {
            for i in 0..4 {
                rhs[i] += row[i] * value;
                for j in 0..4 {
                    normal[i][j] += row[i] * row[j];
                }
            }
        }
    }

    let mut augmented = [[0.0_f64; 5]; 4];
    for i in 0..4 {
        augmented[i][..4].copy_from_slice(&normal[i]);
        augmented[i][4] = rhs[i];
    }

    // Gaussian elimination with partial pivoting.
    for column in 0..4 {
        let pivot = (column..4)
            .max_by(|a, b| {
                augmented[*a][column]
                    .abs()
                    .partial_cmp(&augmented[*b][column].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(column);
        if augmented[pivot][column].abs() <= 1e-12 {
            return None;
        }
        if pivot != column {
            augmented.swap(pivot, column);
        }
        let divisor = augmented[column][column];
        for value in &mut augmented[column][column..] {
            *value /= divisor;
        }
        for row in 0..4 {
            if row == column {
                continue;
            }
            let factor = augmented[row][column];
            if factor == 0.0 {
                continue;
            }
            for i in column..5 {
                augmented[row][i] -= factor * augmented[column][i];
            }
        }
    }

    let a = augmented[0][4] as f32;
    let b = augmented[1][4] as f32;
    let tx = augmented[2][4] as f32;
    let ty = augmented[3][4] as f32;
    Some([a, b, tx, -b, a, ty])
}

/// Applies an affine matrix to a point.
pub fn apply_affine(matrix: &[f32; 6], point: Point) -> Point {
    Point::new(
        matrix[0] * point.x + matrix[1] * point.y + matrix[2],
        matrix[3] * point.x + matrix[4] * point.y + matrix[5],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn letterbox_centers_a_wide_image() {
        let transform = letterbox(1280, 640, 640);

        assert_eq!(transform.resized_w, 640);
        assert_eq!(transform.resized_h, 320);
        close(transform.scale, 0.5, 1e-6);
        close(transform.pad_x, 0.0, 1e-6);
        close(transform.pad_y, 160.0, 1e-6);
    }

    #[test]
    fn letterbox_maps_input_boxes_back_to_source_pixels() {
        let transform = letterbox(1280, 640, 640);
        // A 20×20 box at input (100, 180) sits at source (200, 40) after
        // removing the 160px vertical pad and dividing by 0.5.
        let source = map_rect_to_source(Rect::new(100.0, 180.0, 20.0, 20.0), &transform);

        close(source.x, 200.0, 1e-4);
        close(source.y, 40.0, 1e-4);
        close(source.w, 40.0, 1e-4);
        close(source.h, 40.0, 1e-4);
    }

    #[test]
    fn letterbox_maps_landmarks_back_to_source_pixels() {
        let transform = letterbox(1280, 640, 640);
        let points = [
            Point::new(100.0, 180.0),
            Point::new(120.0, 180.0),
            Point::new(110.0, 200.0),
            Point::new(100.0, 220.0),
            Point::new(120.0, 220.0),
        ];
        let mapped = map_landmarks_to_source(&points, &transform);

        close(mapped[0].x, 200.0, 1e-4);
        close(mapped[0].y, 40.0, 1e-4);
        close(mapped[4].x, 240.0, 1e-4);
        close(mapped[4].y, 120.0, 1e-4);
    }

    #[test]
    fn iou_is_zero_for_disjoint_and_one_for_identical_boxes() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 20.0, 10.0, 10.0);
        assert_eq!(iou(a, b), 0.0);
        close(iou(a, a), 1.0, 1e-6);
    }

    #[test]
    fn nms_keeps_the_highest_scoring_overlap() {
        let detections = vec![
            Detection {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                score: 0.9,
                landmarks: [Point::new(0.0, 0.0); 5],
            },
            Detection {
                rect: Rect::new(1.0, 1.0, 10.0, 10.0),
                score: 0.8,
                landmarks: [Point::new(0.0, 0.0); 5],
            },
            Detection {
                rect: Rect::new(50.0, 50.0, 10.0, 10.0),
                score: 0.7,
                landmarks: [Point::new(0.0, 0.0); 5],
            },
        ];

        let kept = nms(detections, 0.4);
        assert_eq!(kept.len(), 2);
        close(kept[0].score, 0.9, 1e-6);
        close(kept[1].score, 0.7, 1e-6);
    }

    #[test]
    fn cosine_distance_is_zero_for_identical_and_one_for_orthogonal() {
        close(cosine_distance(&[1.0, 0.0], &[1.0, 0.0]), 0.0, 1e-6);
        close(cosine_distance(&[1.0, 0.0], &[0.0, 1.0]), 1.0, 1e-6);
        close(cosine_distance(&[0.0, 0.0], &[1.0, 0.0]), 1.0, 1e-6);
    }

    #[test]
    fn l2_normalize_makes_a_unit_vector() {
        let mut vector = [3.0, 4.0];
        assert!(l2_normalize(&mut vector));
        close(vector[0], 0.6, 1e-6);
        close(vector[1], 0.8, 1e-6);
    }

    #[test]
    fn l2_normalize_rejects_a_zero_vector() {
        let mut vector = [0.0, 0.0];
        assert!(!l2_normalize(&mut vector));
    }

    #[test]
    fn similarity_transform_maps_a_template_onto_itself() {
        let template = [
            Point::new(38.2946, 51.6963),
            Point::new(73.5318, 51.5014),
            Point::new(56.0252, 71.7366),
            Point::new(41.5493, 92.3655),
            Point::new(70.7299, 92.2041),
        ];

        let matrix = similarity_transform(&template, &template).unwrap();
        for point in template {
            let mapped = apply_affine(&matrix, point);
            close(mapped.x, point.x, 1e-3);
            close(mapped.y, point.y, 1e-3);
        }
    }

    #[test]
    fn similarity_transform_recovers_a_known_translation_and_scale() {
        let src = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(5.0, 5.0),
            Point::new(0.0, 10.0),
            Point::new(10.0, 10.0),
        ];
        let dst = src.map(|point| Point::new(point.x * 2.0 + 7.0, point.y * 2.0 + 9.0));

        let matrix = similarity_transform(&src, &dst).unwrap();
        for (source, expected) in src.iter().zip(dst.iter()) {
            let mapped = apply_affine(&matrix, *source);
            close(mapped.x, expected.x, 1e-5);
            close(mapped.y, expected.y, 1e-5);
        }
    }
}
