//! Person segmentation (PP-HumanSeg, opencv_zoo, Apache-2.0) used by the
//! background effects. The model gives a coarse 192x192 person probability
//! map that is upsampled bilinearly when compositing.

use crate::models::{load_model, run, ModelError, Runnable};
use crate::ImageView;
use std::path::Path;

pub const SEGMENTATION_MODEL_FILE: &str = "human_segmentation_pphumanseg_2023mar.onnx";

const INPUT: usize = 192;
/// The model was trained on inputs normalised to [-1, 1].
const NORMALISE_MEAN: f32 = 0.5;
const NORMALISE_STD: f32 = 0.5;

/// Person probability per pixel, 0 = background, 1 = person.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub person: Vec<f32>,
}

impl Mask {
    pub fn filled(width: usize, height: usize, value: f32) -> Self {
        Self { width, height, person: vec![value; width * height] }
    }

    /// Bilinear lookup with `u`/`v` in 0..=1.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let x = (u * (self.width - 1) as f32).clamp(0.0, (self.width - 1) as f32);
        let y = (v * (self.height - 1) as f32).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |px: usize, py: usize| self.person[py * self.width + px];
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Exponential smoothing against flicker: `self = self * (1-w) + newer * w`.
    pub fn blend_towards(&mut self, newer: &Mask, weight: f32) {
        if self.person.len() != newer.person.len() {
            *self = newer.clone();
            return;
        }
        for (old, new) in self.person.iter_mut().zip(&newer.person) {
            *old += (new - *old) * weight;
        }
    }
}

pub struct PersonSegmenter {
    model: Runnable,
}

impl PersonSegmenter {
    pub fn load(dir: &Path) -> Result<Self, ModelError> {
        Ok(Self { model: load_model(&dir.join(SEGMENTATION_MODEL_FILE), [1, 3, INPUT, INPUT])? })
    }

    /// Estimates the person mask of `image` (resized internally to 192x192).
    pub fn mask(&self, image: &ImageView) -> Result<Mask, ModelError> {
        // NCHW, channel-planar.
        let mut data = vec![0.0f32; 3 * INPUT * INPUT];
        for y in 0..INPUT {
            for x in 0..INPUT {
                let sx = (x as f32 + 0.5) / INPUT as f32 * image.width as f32 - 0.5;
                let sy = (y as f32 + 0.5) / INPUT as f32 * image.height as f32 - 0.5;
                let rgb = image.sample(sx.max(0.0), sy.max(0.0));
                for (channel, value) in rgb.iter().enumerate() {
                    data[channel * INPUT * INPUT + y * INPUT + x] = (value - NORMALISE_MEAN) / NORMALISE_STD;
                }
            }
        }
        let outputs = run(&self.model, [1, 3, INPUT, INPUT], data)?;
        let logits = outputs
            .iter()
            .find(|o| o.len() == 2 * INPUT * INPUT)
            .ok_or_else(|| ModelError::new("unexpected segmentation output shape"))?;
        let (background, person) = logits.split_at(INPUT * INPUT);
        let probabilities = background
            .iter()
            .zip(person)
            .map(|(b, p)| 1.0 / (1.0 + (b - p).exp())) // softmax of two classes
            .collect();
        Ok(Mask { width: INPUT, height: INPUT, person: probabilities })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_interpolates_between_pixels() {
        let mask = Mask { width: 2, height: 1, person: vec![0.0, 1.0] };
        assert!((mask.sample(0.0, 0.0) - 0.0).abs() < 1e-6);
        assert!((mask.sample(1.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((mask.sample(0.5, 0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn blend_moves_toward_the_new_mask() {
        let mut old = Mask::filled(2, 2, 0.0);
        old.blend_towards(&Mask::filled(2, 2, 1.0), 0.25);
        assert!(old.person.iter().all(|v| (v - 0.25).abs() < 1e-6));
    }
}
