//! SCRFD-2.5G output decoding.
//!
//! The model returns nine tensors, three per feature-map stride (8, 16, 32):
//! scores, box distances and five landmark distances. The order is the one the
//! exported `buffalo_m` model uses, confirmed from its output shapes:
//! `[score_8, score_16, score_32, bbox_8, bbox_16, bbox_32, kps_8, kps_16,
//! kps_32]`.

use super::geometry::{Detection, Point, Rect};

pub const STRIDES: [usize; 3] = [8, 16, 32];
pub const ANCHORS_PER_CELL: usize = 2;

/// Decodes one stride's raw tensors into input-space detections.
pub fn decode_scrfd(
    scores: &[&[f32]; 3],
    bboxes: &[&[f32]; 3],
    landmarks: &[&[f32]; 3],
    input_w: usize,
    input_h: usize,
    score_threshold: f32,
) -> Vec<Detection> {
    let mut decoded = Vec::new();

    for (level, stride) in STRIDES.iter().copied().enumerate() {
        let grid_w = input_w / stride;
        let grid_h = input_h / stride;
        let anchors = grid_w * grid_h * ANCHORS_PER_CELL;
        let scores = scores[level];
        let bboxes = bboxes[level];
        let landmarks = landmarks[level];

        for index in 0..anchors {
            let Some(score) = scores.get(index).copied() else {
                break;
            };
            if score < score_threshold {
                continue;
            }

            let cell = index / ANCHORS_PER_CELL;
            let center_x = (cell % grid_w) as f32 * stride as f32;
            let center_y = (cell / grid_w) as f32 * stride as f32;

            let Some(box_values) = bboxes.get(index * 4..index * 4 + 4) else {
                break;
            };
            let left = center_x - box_values[0] * stride as f32;
            let top = center_y - box_values[1] * stride as f32;
            let right = center_x + box_values[2] * stride as f32;
            let bottom = center_y + box_values[3] * stride as f32;

            let Some(landmark_values) = landmarks.get(index * 10..index * 10 + 10) else {
                break;
            };
            let mut points = [Point::new(0.0, 0.0); 5];
            for (point_index, point) in points.iter_mut().enumerate() {
                point.x = center_x + landmark_values[point_index * 2] * stride as f32;
                point.y = center_y + landmark_values[point_index * 2 + 1] * stride as f32;
            }

            decoded.push(Detection {
                rect: Rect::new(left, top, right - left, bottom - top),
                score,
                landmarks: points,
            });
        }
    }

    decoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_first_anchor_of_the_first_stride() {
        let mut scores = [
            vec![0.0_f32; 12_800],
            vec![0.0_f32; 3_200],
            vec![0.0_f32; 800],
        ];
        let mut bboxes = [
            vec![0.0_f32; 51_200],
            vec![0.0_f32; 12_800],
            vec![0.0_f32; 3_200],
        ];
        let mut landmarks = [
            vec![0.0_f32; 128_000],
            vec![0.0_f32; 32_000],
            vec![0.0_f32; 8_000],
        ];

        scores[0][0] = 0.9;
        bboxes[0][..4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        landmarks[0][..10].copy_from_slice(&[0.5, 0.5, 1.0, 1.0, 1.5, 1.5, 2.0, 2.0, 2.5, 2.5]);

        let decoded = decode_scrfd(
            &[
                scores[0].as_slice(),
                scores[1].as_slice(),
                scores[2].as_slice(),
            ],
            &[
                bboxes[0].as_slice(),
                bboxes[1].as_slice(),
                bboxes[2].as_slice(),
            ],
            &[
                landmarks[0].as_slice(),
                landmarks[1].as_slice(),
                landmarks[2].as_slice(),
            ],
            640,
            640,
            0.5,
        );

        assert_eq!(decoded.len(), 1);
        let detection = &decoded[0];
        assert_eq!(detection.rect, Rect::new(-8.0, -8.0, 16.0, 16.0));
        assert_eq!(detection.landmarks[0], Point::new(4.0, 4.0));
        assert_eq!(detection.landmarks[4], Point::new(20.0, 20.0));
    }

    #[test]
    fn skips_scores_below_the_threshold() {
        let mut scores = [
            vec![0.0_f32; 12_800],
            vec![0.0_f32; 3_200],
            vec![0.0_f32; 800],
        ];
        let bboxes = [
            vec![0.0_f32; 51_200],
            vec![0.0_f32; 12_800],
            vec![0.0_f32; 3_200],
        ];
        let landmarks = [
            vec![0.0_f32; 128_000],
            vec![0.0_f32; 32_000],
            vec![0.0_f32; 8_000],
        ];
        scores[0][0] = 0.49;

        let decoded = decode_scrfd(
            &[
                scores[0].as_slice(),
                scores[1].as_slice(),
                scores[2].as_slice(),
            ],
            &[
                bboxes[0].as_slice(),
                bboxes[1].as_slice(),
                bboxes[2].as_slice(),
            ],
            &[
                landmarks[0].as_slice(),
                landmarks[1].as_slice(),
                landmarks[2].as_slice(),
            ],
            640,
            640,
            0.5,
        );

        assert!(decoded.is_empty());
    }
}
