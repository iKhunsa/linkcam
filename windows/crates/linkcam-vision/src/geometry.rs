//! Hand landmark layout and small geometry helpers.

pub const HAND_LANDMARKS: usize = 21;

/// MediaPipe hand landmark indices used by the classifier.
pub mod idx {
    pub const WRIST: usize = 0;
    pub const THUMB_IP: usize = 3;
    pub const THUMB_TIP: usize = 4;
    pub const INDEX_MCP: usize = 5;
    pub const INDEX_PIP: usize = 6;
    pub const INDEX_TIP: usize = 8;
    pub const MIDDLE_MCP: usize = 9;
    pub const MIDDLE_PIP: usize = 10;
    pub const MIDDLE_TIP: usize = 12;
    pub const RING_PIP: usize = 14;
    pub const RING_TIP: usize = 16;
    pub const PINKY_PIP: usize = 18;
    pub const PINKY_TIP: usize = 20;
}

/// A point in normalised image coordinates (0..1, y grows downwards).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Landmark {
    pub x: f32,
    pub y: f32,
}

impl Landmark {
    pub fn distance(self, other: Landmark) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// One detected hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Hand {
    pub landmarks: [Landmark; HAND_LANDMARKS],
    pub confidence: f32,
}

impl Hand {
    pub fn wrist(&self) -> Landmark {
        self.landmarks[idx::WRIST]
    }

    /// Wrist-to-middle-knuckle distance: a scale reference independent of camera distance.
    pub fn size(&self) -> f32 {
        self.landmarks[idx::WRIST].distance(self.landmarks[idx::MIDDLE_MCP]).max(1e-4)
    }
}
