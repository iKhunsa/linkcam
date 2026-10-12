//! Standard UVC controls through `IAMCameraControl` / `IAMVideoProcAmp`.

use linkcam_core::{CameraError, ControlValue, ImageControl, Result};
use windows::core::Interface;
use windows::Win32::Media::DirectShow::{
    CameraControl_Exposure, CameraControl_Flags_Auto, CameraControl_Flags_Manual, CameraControl_Pan,
    CameraControl_Tilt, CameraControl_Zoom, IAMCameraControl, IAMVideoProcAmp, IBaseFilter,
    VideoProcAmp_Brightness, VideoProcAmp_Contrast, VideoProcAmp_Flags_Auto, VideoProcAmp_Flags_Manual,
    VideoProcAmp_Saturation, VideoProcAmp_Sharpness, VideoProcAmp_WhiteBalance,
};

/// One of the camera-control properties used for digital pan/tilt/zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtzAxis {
    Pan,
    Tilt,
    Zoom,
}

impl PtzAxis {
    fn property(self) -> i32 {
        match self {
            Self::Pan => CameraControl_Pan.0,
            Self::Tilt => CameraControl_Tilt.0,
            Self::Zoom => CameraControl_Zoom.0,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Pan => "pan",
            Self::Tilt => "tilt",
            Self::Zoom => "zoom",
        }
    }
}

pub struct StandardControls {
    camera: Option<IAMCameraControl>,
    proc_amp: Option<IAMVideoProcAmp>,
}

fn proc_amp_property(control: ImageControl) -> Option<i32> {
    match control {
        ImageControl::Brightness => Some(VideoProcAmp_Brightness.0),
        ImageControl::Contrast => Some(VideoProcAmp_Contrast.0),
        ImageControl::Saturation => Some(VideoProcAmp_Saturation.0),
        ImageControl::Sharpness => Some(VideoProcAmp_Sharpness.0),
        ImageControl::WhiteBalance => Some(VideoProcAmp_WhiteBalance.0),
        ImageControl::Exposure => None,
    }
}

impl StandardControls {
    pub fn new(filter: &IBaseFilter) -> Self {
        Self { camera: filter.cast().ok(), proc_amp: filter.cast().ok() }
    }

    fn camera(&self, name: &'static str) -> Result<&IAMCameraControl> {
        self.camera.as_ref().ok_or(CameraError::ControlUnavailable(name))
    }

    /// Current value and limits of a pan/tilt/zoom axis.
    pub fn ptz(&self, axis: PtzAxis) -> Result<ControlValue> {
        let camera = self.camera(axis.name())?;
        let (mut min, mut max, mut step, mut default, mut caps) = (0, 0, 0, 0, 0);
        let (mut value, mut flags) = (0, 0);
        unsafe {
            camera
                .GetRange(axis.property(), &mut min, &mut max, &mut step, &mut default, &mut caps)
                .map_err(|_| CameraError::ControlUnavailable(axis.name()))?;
            camera
                .Get(axis.property(), &mut value, &mut flags)
                .map_err(|e| CameraError::ReadFailed(e.to_string()))?;
        }
        Ok(ControlValue { value, min, max, step, default, auto: flags & CameraControl_Flags_Auto.0 != 0 })
    }

    pub fn set_ptz(&self, axis: PtzAxis, value: i32) -> Result<()> {
        let camera = self.camera(axis.name())?;
        unsafe { camera.Set(axis.property(), value, CameraControl_Flags_Manual.0) }
            .map_err(|e| CameraError::WriteFailed(format!("{}: {e}", axis.name())))
    }

    pub fn image(&self, control: ImageControl) -> Result<ControlValue> {
        let (mut min, mut max, mut step, mut default, mut caps) = (0, 0, 0, 0, 0);
        let (mut value, mut flags) = (0, 0);
        match proc_amp_property(control) {
            Some(property) => {
                let amp = self.proc_amp.as_ref().ok_or(CameraError::ControlUnavailable(control.label()))?;
                unsafe {
                    amp.GetRange(property, &mut min, &mut max, &mut step, &mut default, &mut caps)
                        .map_err(|_| CameraError::ControlUnavailable(control.label()))?;
                    amp.Get(property, &mut value, &mut flags)
                        .map_err(|e| CameraError::ReadFailed(e.to_string()))?;
                }
                Ok(ControlValue {
                    value,
                    min,
                    max,
                    step,
                    default,
                    auto: flags & VideoProcAmp_Flags_Auto.0 != 0,
                })
            }
            None => {
                let camera = self.camera(control.label())?;
                unsafe {
                    camera
                        .GetRange(
                            CameraControl_Exposure.0,
                            &mut min,
                            &mut max,
                            &mut step,
                            &mut default,
                            &mut caps,
                        )
                        .map_err(|_| CameraError::ControlUnavailable(control.label()))?;
                    camera
                        .Get(CameraControl_Exposure.0, &mut value, &mut flags)
                        .map_err(|e| CameraError::ReadFailed(e.to_string()))?;
                }
                Ok(ControlValue {
                    value,
                    min,
                    max,
                    step,
                    default,
                    auto: flags & CameraControl_Flags_Auto.0 != 0,
                })
            }
        }
    }

    pub fn set_image(&self, control: ImageControl, value: i32, auto: bool) -> Result<()> {
        let write_err =
            |e: windows::core::Error| CameraError::WriteFailed(format!("{}: {e}", control.label()));
        match proc_amp_property(control) {
            Some(property) => {
                let amp = self.proc_amp.as_ref().ok_or(CameraError::ControlUnavailable(control.label()))?;
                let flags = if auto { VideoProcAmp_Flags_Auto.0 } else { VideoProcAmp_Flags_Manual.0 };
                unsafe { amp.Set(property, value, flags) }.map_err(write_err)
            }
            None => {
                let camera = self.camera(control.label())?;
                let flags = if auto { CameraControl_Flags_Auto.0 } else { CameraControl_Flags_Manual.0 };
                unsafe { camera.Set(CameraControl_Exposure.0, value, flags) }.map_err(write_err)
            }
        }
    }
}
