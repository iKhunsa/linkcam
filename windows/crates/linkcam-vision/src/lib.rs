//! Host-side hand-gesture recognition.
//!
//! Pipeline: RGBA frame -> palm detector -> rotated ROI -> hand landmarks
//! (MediaPipe models run with `tract`) -> `classify` -> `Dispatcher`
//! (hold / cooldown / swipe) -> bindings to fire.
//!
//! Everything except `HandModels` is pure and testable without a camera.

pub mod classifier;
pub mod dispatcher;
pub mod effects;
pub mod geometry;
pub mod models;
pub mod segmentation;

pub use classifier::classify;
pub use dispatcher::{Dispatcher, Fired};
pub use geometry::{Hand, Landmark, HAND_LANDMARKS};
pub use models::{HandModels, ModelError, Roi};
pub use segmentation::{Mask, PersonSegmenter};

/// Borrowed RGBA image (row-major, 4 bytes per pixel).
#[derive(Debug, Clone, Copy)]
pub struct ImageView<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
}

impl ImageView<'_> {
    /// Bilinear sample of the RGB channels at pixel coordinates; black outside.
    pub fn sample(&self, x: f32, y: f32) -> [f32; 3] {
        if x < 0.0 || y < 0.0 || x > (self.width - 1) as f32 || y > (self.height - 1) as f32 {
            return [0.0; 3];
        }
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |px: usize, py: usize, c: usize| self.rgba[(py * self.width + px) * 4 + c] as f32 / 255.0;
        let mut out = [0.0; 3];
        for (c, value) in out.iter_mut().enumerate() {
            let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
            let bottom = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
            *value = top * (1.0 - fy) + bottom * fy;
        }
        out
    }
}
