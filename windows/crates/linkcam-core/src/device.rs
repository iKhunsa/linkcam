//! The abstraction the UI, CLI and gesture engine program against.

use crate::error::{CameraError, Result};
use crate::modes::VideoMode;
use crate::units::{AngleUnits, PtzLimits, ZoomUnits};

/// Snapshot of everything the controller shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraState {
    pub pan: AngleUnits,
    pub tilt: AngleUnits,
    pub zoom: ZoomUnits,
    /// `None` when the mode selector is unavailable on this firmware.
    pub mode: Option<VideoMode>,
}

/// Image adjustments exposed through the standard UVC processing unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageControl {
    Brightness,
    Contrast,
    Saturation,
    Sharpness,
    WhiteBalance,
    Exposure,
}

impl ImageControl {
    pub const ALL: [Self; 6] = [
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Sharpness,
        Self::WhiteBalance,
        Self::Exposure,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Brightness => "Brightness",
            Self::Contrast => "Contrast",
            Self::Saturation => "Saturation",
            Self::Sharpness => "Sharpness",
            Self::WhiteBalance => "White balance",
            Self::Exposure => "Exposure",
        }
    }
}

/// Current value and limits of one image control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlValue {
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub default: i32,
    pub auto: bool,
}

pub trait CameraDevice {
    fn limits(&self) -> PtzLimits;
    /// Whether pan/tilt actually move the view. The Link 2C advertises the
    /// controls but its firmware ignores them (see `docs/xu-map-link2c.md`).
    fn supports_pan_tilt(&self) -> bool {
        true
    }
    fn read_state(&mut self) -> Result<CameraState>;
    fn set_zoom(&mut self, zoom: ZoomUnits) -> Result<CameraState>;
    fn set_pan_tilt(&mut self, pan: AngleUnits, tilt: AngleUnits) -> Result<CameraState>;
    fn set_mode(&mut self, mode: VideoMode) -> Result<CameraState>;
    fn image_control(&mut self, control: ImageControl) -> Result<ControlValue>;
    fn set_image_control(&mut self, control: ImageControl, value: i32, auto: bool) -> Result<()>;
}

/// In-memory camera used by tests and by `--demo` mode of the app.
pub struct FakeCamera {
    state: CameraState,
    limits: PtzLimits,
    controls: Vec<(ImageControl, ControlValue)>,
}

impl FakeCamera {
    pub fn new() -> Self {
        let limits = PtzLimits::LINK_2;
        let control = ControlValue { value: 50, min: 0, max: 100, step: 1, default: 50, auto: false };
        Self {
            state: CameraState {
                pan: AngleUnits::ZERO,
                tilt: AngleUnits::ZERO,
                zoom: ZoomUnits::ONE_X,
                mode: Some(VideoMode::Normal),
            },
            limits,
            controls: ImageControl::ALL.into_iter().map(|c| (c, control)).collect(),
        }
    }
}

impl Default for FakeCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraDevice for FakeCamera {
    fn limits(&self) -> PtzLimits {
        self.limits
    }

    fn read_state(&mut self) -> Result<CameraState> {
        Ok(self.state)
    }

    fn set_zoom(&mut self, zoom: ZoomUnits) -> Result<CameraState> {
        self.state.zoom = ZoomUnits::from_raw_clamped(zoom.raw() as i64, self.limits.zoom);
        Ok(self.state)
    }

    fn set_pan_tilt(&mut self, pan: AngleUnits, tilt: AngleUnits) -> Result<CameraState> {
        self.state.pan = AngleUnits::from_raw_clamped(pan.raw() as i64, self.limits.pan);
        self.state.tilt = AngleUnits::from_raw_clamped(tilt.raw() as i64, self.limits.tilt);
        Ok(self.state)
    }

    fn set_mode(&mut self, mode: VideoMode) -> Result<CameraState> {
        self.state.mode = Some(mode);
        Ok(self.state)
    }

    fn image_control(&mut self, control: ImageControl) -> Result<ControlValue> {
        self.controls
            .iter()
            .find(|(c, _)| *c == control)
            .map(|(_, v)| *v)
            .ok_or(CameraError::ControlUnavailable(control.label()))
    }

    fn set_image_control(&mut self, control: ImageControl, value: i32, auto: bool) -> Result<()> {
        let slot = self
            .controls
            .iter_mut()
            .find(|(c, _)| *c == control)
            .ok_or(CameraError::ControlUnavailable(control.label()))?;
        slot.1.value = value.clamp(slot.1.min, slot.1.max);
        slot.1.auto = auto;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_camera_clamps_and_reports_readback() {
        let mut cam = FakeCamera::new();
        let state = cam.set_zoom(ZoomUnits::from_raw(900)).unwrap();
        assert_eq!(state.zoom.raw(), 400);
        let state = cam.set_mode(VideoMode::Whiteboard).unwrap();
        assert_eq!(state.mode, Some(VideoMode::Whiteboard));
    }

    #[test]
    fn fake_camera_image_controls_are_clamped() {
        let mut cam = FakeCamera::new();
        cam.set_image_control(ImageControl::Brightness, 500, false).unwrap();
        assert_eq!(cam.image_control(ImageControl::Brightness).unwrap().value, 100);
    }
}
