//! Hardware-independent domain of LinkCam: units, protocol payloads,
//! camera modes, gestures and configuration.
//!
//! Nothing here touches Windows APIs, so everything is unit-testable.

pub mod config;
pub mod device;
pub mod error;
pub mod format;
pub mod gesture;
pub mod modes;
pub mod protocol;
pub mod units;

pub use device::{CameraDevice, CameraState, ControlValue, FakeCamera, ImageControl};
pub use error::{CameraError, Result};
pub use format::VideoFormat;
pub use modes::VideoMode;
pub use units::{AngleUnits, PtzLimits, ZoomUnits};
