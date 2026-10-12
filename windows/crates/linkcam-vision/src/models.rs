//! MediaPipe palm detector + hand landmark models executed with `tract`.
//!
//! Pre/post-processing follows MediaPipe's graph: letterboxed 192x192 palm
//! detection -> rotated square ROI -> 224x224 landmark model -> landmarks
//! mapped back to the image. Models: opencv_zoo `palm_detection_mediapipe`
//! and `handpose_estimation_mediapipe` (Apache-2.0).

use crate::geometry::{Hand, Landmark, HAND_LANDMARKS};
use crate::ImageView;
use std::fmt;
use std::path::Path;
use tract_onnx::prelude::*;

pub const PALM_MODEL_FILE: &str = "palm_detection_mediapipe_2023feb.onnx";
pub const LANDMARK_MODEL_FILE: &str = "handpose_estimation_mediapipe_2023feb.onnx";

const PALM_INPUT: usize = 192;
const LANDMARK_INPUT: usize = 224;
const PALM_ANCHORS: usize = 2016;
const PALM_OUTPUT_COLUMNS: usize = 18;
/// Palm score below this means "no hand".
const PALM_SCORE_THRESHOLD: f32 = 0.55;
/// Hand-presence score below this rejects the landmark result.
const HAND_PRESENCE_THRESHOLD: f32 = 0.6;
/// Palm keypoints -> hand ROI transform (opencv_zoo: shift up 0.4, enlarge 3x
/// the bounding box of the 7 palm keypoints, taken in the upright frame).
const ROI_SCALE: f32 = 3.0;
const ROI_SHIFT_Y: f32 = -0.4;
const PALM_KEYPOINTS: usize = 7;
/// ROI multipliers tried per palm, and the presence that ends the search early.
const ROI_SCALES_TRIED: [f32; 4] = [1.0, 1.3, 1.6, 2.0];
const EARLY_EXIT_PRESENCE: f32 = 0.85;

#[derive(Debug)]
pub struct ModelError(String);

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ModelError {}

impl From<TractError> for ModelError {
    fn from(e: TractError) -> Self {
        Self(e.to_string())
    }
}

type Runnable = Arc<TypedRunnableModel>;

pub struct HandModels {
    palm: Runnable,
    landmarks: Runnable,
    anchors: Vec<(f32, f32)>,
}

/// A palm detection in normalised image coordinates.
struct Palm {
    /// Seven palm keypoints (0 = wrist, 2 = middle-finger base), normalised.
    keypoints: [(f32, f32); PALM_KEYPOINTS],
}

impl HandModels {
    /// Loads both models from `dir`.
    pub fn load(dir: &Path) -> Result<Self, ModelError> {
        Ok(Self {
            palm: load_model(&dir.join(PALM_MODEL_FILE), PALM_INPUT)?,
            landmarks: load_model(&dir.join(LANDMARK_MODEL_FILE), LANDMARK_INPUT)?,
            anchors: palm_anchors(),
        })
    }

    /// Finds the most confident hand in `image`.
    pub fn detect(&self, image: &ImageView) -> Result<Option<Hand>, ModelError> {
        let Some(palm) = self.detect_palm(image)? else { return Ok(None) };
        self.landmarks_for(image, &palm)
    }

    fn detect_palm(&self, image: &ImageView) -> Result<Option<Palm>, ModelError> {
        // Letterbox the frame into a square input, keeping the aspect ratio.
        let side = image.width.max(image.height) as f32;
        let pad_x = (side - image.width as f32) / 2.0;
        let pad_y = (side - image.height as f32) / 2.0;
        let mut input = Vec::with_capacity(PALM_INPUT * PALM_INPUT * 3);
        for v in 0..PALM_INPUT {
            for u in 0..PALM_INPUT {
                let sx = (u as f32 + 0.5) / PALM_INPUT as f32 * side - pad_x - 0.5;
                let sy = (v as f32 + 0.5) / PALM_INPUT as f32 * side - pad_y - 0.5;
                input.extend_from_slice(&image.sample(sx, sy));
            }
        }
        let outputs = run(&self.palm, input, PALM_INPUT)?;
        let (boxes, scores) = split_palm_outputs(&outputs)?;

        let mut best: Option<(usize, f32)> = None;
        for (i, raw) in scores.iter().enumerate().take(PALM_ANCHORS) {
            let score = 1.0 / (1.0 + (-raw).exp());
            if score > PALM_SCORE_THRESHOLD && best.is_none_or(|(_, s)| score > s) {
                best = Some((i, score));
            }
        }
        let Some((i, _score)) = best else { return Ok(None) };

        let row = &boxes[i * PALM_OUTPUT_COLUMNS..(i + 1) * PALM_OUTPUT_COLUMNS];
        let (ax, ay) = self.anchors[i];
        let input_size = PALM_INPUT as f32;
        // Normalised letterbox coordinates -> normalised image coordinates.
        let to_image = |nx: f32, ny: f32| {
            ((nx * side - pad_x) / image.width as f32, (ny * side - pad_y) / image.height as f32)
        };
        let mut keypoints = [(0.0, 0.0); PALM_KEYPOINTS];
        for (k, point) in keypoints.iter_mut().enumerate() {
            *point = to_image(row[4 + 2 * k] / input_size + ax, row[5 + 2 * k] / input_size + ay);
        }
        Ok(Some(Palm { keypoints }))
    }

    /// The landmark model is very sensitive to ROI size, so a few scales are
    /// tried and the most confident result wins (early exit when clearly a hand).
    fn landmarks_for(&self, image: &ImageView, palm: &Palm) -> Result<Option<Hand>, ModelError> {
        let base = roi_from_palm(palm, image.width as f32, image.height as f32);
        let mut best: Option<Hand> = None;
        for scale in ROI_SCALES_TRIED {
            let roi = Roi { side: base.side * scale, ..base };
            if let Some(hand) = self.landmarks_in_roi(image, &roi)? {
                if hand.confidence >= EARLY_EXIT_PRESENCE {
                    return Ok(Some(hand));
                }
                if best.as_ref().is_none_or(|b| hand.confidence > b.confidence) {
                    best = Some(hand);
                }
            }
        }
        Ok(best)
    }

    /// Runs only the landmark model on a given region of interest (pixels).
    pub fn landmarks_in_roi(&self, image: &ImageView, roi: &Roi) -> Result<Option<Hand>, ModelError> {
        let (w, h) = (image.width as f32, image.height as f32);
        let (sin, cos) = roi.rotation.sin_cos();
        let to_source = |lx: f32, ly: f32| {
            (roi.center_x + (lx * cos - ly * sin) * roi.side, roi.center_y + (lx * sin + ly * cos) * roi.side)
        };

        let mut input = Vec::with_capacity(LANDMARK_INPUT * LANDMARK_INPUT * 3);
        for v in 0..LANDMARK_INPUT {
            for u in 0..LANDMARK_INPUT {
                let lx = (u as f32 + 0.5) / LANDMARK_INPUT as f32 - 0.5;
                let ly = (v as f32 + 0.5) / LANDMARK_INPUT as f32 - 0.5;
                let (sx, sy) = to_source(lx, ly);
                input.extend_from_slice(&image.sample(sx, sy));
            }
        }
        let outputs = run(&self.landmarks, input, LANDMARK_INPUT)?;
        let (coords, presence) = split_landmark_outputs(&outputs)?;
        if presence < HAND_PRESENCE_THRESHOLD {
            return Ok(None);
        }

        let mut landmarks = [Landmark::default(); HAND_LANDMARKS];
        for (i, point) in landmarks.iter_mut().enumerate() {
            let (px, py) = to_source(
                coords[i * 3] / LANDMARK_INPUT as f32 - 0.5,
                coords[i * 3 + 1] / LANDMARK_INPUT as f32 - 0.5,
            );
            *point = Landmark { x: px / w, y: py / h };
        }
        Ok(Some(Hand { landmarks, confidence: presence }))
    }
}

/// Rotated square region of the image fed to the landmark model (pixels, radians).
#[derive(Debug, Clone, Copy)]
pub struct Roi {
    pub center_x: f32,
    pub center_y: f32,
    pub side: f32,
    pub rotation: f32,
}

fn roi_from_palm(palm: &Palm, w: f32, h: f32) -> Roi {
    // Work in pixels so rotation is not distorted by the aspect ratio.
    let px: Vec<(f32, f32)> = palm.keypoints.iter().map(|&(x, y)| (x * w, y * h)).collect();
    let (wrist, middle_base) = (px[0], px[2]);
    let rotation = normalize_angle(
        std::f32::consts::FRAC_PI_2 - (-(middle_base.1 - wrist.1)).atan2(middle_base.0 - wrist.0),
    );
    let (sin, cos) = rotation.sin_cos();

    // Bounding box of the keypoints in the hand's upright frame.
    let (mut min, mut max) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
    for &(x, y) in &px {
        let (ux, uy) = (x * cos + y * sin, -x * sin + y * cos);
        min = (min.0.min(ux), min.1.min(uy));
        max = (max.0.max(ux), max.1.max(uy));
    }
    let (box_w, box_h) = (max.0 - min.0, max.1 - min.1);
    let (ux, uy) = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0 + ROI_SHIFT_Y * box_h);
    Roi {
        center_x: ux * cos - uy * sin,
        center_y: ux * sin + uy * cos,
        side: box_w.max(box_h) * ROI_SCALE,
        rotation,
    }
}

fn load_model(path: &Path, input_size: usize) -> Result<Runnable, ModelError> {
    tract_onnx::onnx()
        .model_for_path(path)
        .map_err(|e| ModelError(format!("{}: {e}", path.display())))?
        .with_input_fact(0, f32::fact([1, input_size, input_size, 3]).into())?
        .into_optimized()?
        .into_runnable()
        .map_err(Into::into)
}

fn run(model: &Runnable, data: Vec<f32>, size: usize) -> Result<Vec<Vec<f32>>, ModelError> {
    let input = tract_ndarray::Array4::from_shape_vec((1, size, size, 3), data)
        .map_err(|e| ModelError(e.to_string()))?;
    let outputs = model.run(tvec!(Tensor::from(input).into_tvalue()))?;
    outputs.iter().map(|t| Ok(t.to_plain_array_view::<f32>()?.iter().copied().collect())).collect()
}

/// The palm model has two outputs: boxes+keypoints `[1,2016,18]` and scores `[1,2016,1]`.
fn split_palm_outputs(outputs: &[Vec<f32>]) -> Result<(&[f32], &[f32]), ModelError> {
    let boxes = outputs.iter().find(|o| o.len() == PALM_ANCHORS * PALM_OUTPUT_COLUMNS);
    let scores = outputs.iter().find(|o| o.len() == PALM_ANCHORS);
    boxes
        .zip(scores)
        .map(|(b, s)| (b.as_slice(), s.as_slice()))
        .ok_or_else(|| ModelError("unexpected palm model output shapes".into()))
}

/// The landmark model returns 63 screen coordinates and a presence score.
fn split_landmark_outputs(outputs: &[Vec<f32>]) -> Result<(&[f32], f32), ModelError> {
    let coords = outputs.iter().find(|o| o.len() == HAND_LANDMARKS * 3);
    let presence = outputs.iter().find(|o| o.len() == 1);
    coords
        .zip(presence)
        .map(|(c, p)| (c.as_slice(), p[0]))
        .ok_or_else(|| ModelError("unexpected landmark model output shapes".into()))
}

/// SSD anchors of the 192x192 palm detector: strides 8 (2 per cell) and 16 (6 per cell).
fn palm_anchors() -> Vec<(f32, f32)> {
    let mut anchors = Vec::with_capacity(PALM_ANCHORS);
    for (grid, per_cell) in [(24usize, 2usize), (12, 6)] {
        for row in 0..grid {
            for col in 0..grid {
                let center = ((col as f32 + 0.5) / grid as f32, (row as f32 + 0.5) / grid as f32);
                anchors.extend(std::iter::repeat_n(center, per_cell));
            }
        }
    }
    anchors
}

fn normalize_angle(radians: f32) -> f32 {
    let two_pi = 2.0 * std::f32::consts::PI;
    radians - two_pi * ((radians + std::f32::consts::PI) / two_pi).floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_count_matches_the_model() {
        let anchors = palm_anchors();
        assert_eq!(anchors.len(), PALM_ANCHORS);
        assert_eq!(anchors[0], (0.5 / 24.0, 0.5 / 24.0));
        assert_eq!(anchors[1], anchors[0]);
        assert_eq!(anchors[2], (1.5 / 24.0, 0.5 / 24.0));
    }

    #[test]
    fn angle_is_wrapped_to_pi() {
        let wrapped = normalize_angle(3.0 * std::f32::consts::PI / 2.0);
        assert!((wrapped + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }
}
