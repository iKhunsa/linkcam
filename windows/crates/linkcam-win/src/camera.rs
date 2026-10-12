//! `WinCamera`: the Windows implementation of `CameraDevice`, plus the
//! video stream the firmware needs in order to accept mode changes.

use crate::controls::{PtzAxis, StandardControls};
use crate::discovery::{open_insta360_filter, CameraInfo};
use crate::preview::{Frame, PreviewCapture};
use crate::xu::{XuClient, KNOWN_XU_GUIDS};
use linkcam_core::protocol::{
    mode_from_payload, payload_with_mode, PRIMARY_XU_GUID_INDEX, SELECTOR_VIDEO_MODE,
};
use linkcam_core::units::{AngleRange, ZoomRange};
use linkcam_core::{
    AngleUnits, CameraDevice, CameraError, CameraState, ControlValue, ImageControl, PtzLimits, Result,
    VideoFormat, VideoMode, ZoomUnits,
};
use std::thread::sleep;
use std::time::Duration;
use windows::Win32::Media::DirectShow::IBaseFilter;

/// How long to wait for the firmware to echo an applied mode.
const MODE_POLL_INTERVAL: Duration = Duration::from_millis(200);
const MODE_POLLS: usize = 15;
/// Frames pulled when a stream starts so the firmware sees an active pipeline.
const STREAM_PRIME_FRAMES: usize = 5;
/// Reads attempted when waiting for a full-resolution frame.
const FULL_FRAME_ATTEMPTS: usize = 10;
/// Friendly-name fragment used to find the Media Foundation video source.
const VIDEO_SOURCE_NAME: &str = "Insta360";

pub struct WinCamera {
    info: CameraInfo,
    xu: XuClient,
    controls: StandardControls,
    xu_node: Option<u32>,
    limits: PtzLimits,
    stream: Option<PreviewCapture>,
    preferred_format: Option<VideoFormat>,
    video_source: String,
    formats: Vec<VideoFormat>,
    // Keeps the DirectShow filter (and thus the device) alive; dropped last.
    _filter: IBaseFilter,
}

impl WinCamera {
    pub fn open() -> Result<Self> {
        let (info, filter) = open_insta360_filter()?;
        let xu = XuClient::new(&filter)?;
        let controls = StandardControls::new(&filter);
        let guid = KNOWN_XU_GUIDS[PRIMARY_XU_GUID_INDEX];
        let xu_node = xu
            .extension_nodes()
            .into_iter()
            .find(|&node| xu.access(node, &guid, SELECTOR_VIDEO_MODE).is_some());
        let limits = Self::read_limits(&controls);
        Ok(Self {
            info,
            xu,
            controls,
            xu_node,
            limits,
            stream: None,
            preferred_format: None,
            video_source: String::new(),
            formats: Vec::new(),
            _filter: filter,
        })
    }

    pub fn info(&self) -> &CameraInfo {
        &self.info
    }

    /// Raw value and limits of one pan/tilt/zoom axis, exactly as the driver reports them.
    pub fn ptz_control(&self, axis: PtzAxis) -> Result<ControlValue> {
        self.controls.ptz(axis)
    }

    /// True when the vendor mode selector answered, i.e. modes can be switched.
    pub fn supports_modes(&self) -> bool {
        self.xu_node.is_some()
    }

    /// Starts the video stream if it is not running. Modes only apply while streaming.
    pub fn ensure_streaming(&mut self) -> Result<()> {
        if self.stream.is_none() {
            let source = if self.video_source.is_empty() { VIDEO_SOURCE_NAME } else { &self.video_source };
            let mut stream = PreviewCapture::open(source, self.preferred_format)?;
            self.formats = stream.available_formats().to_vec();
            // The pipeline only runs once frames are being pulled.
            for _ in 0..STREAM_PRIME_FRAMES {
                stream.next_frame()?;
            }
            self.stream = Some(stream);
        }
        Ok(())
    }

    /// Captures from the device whose name contains `name` instead of the camera
    /// (empty restores the default). Used to test the pipeline with a virtual camera.
    pub fn set_video_source(&mut self, name: &str) {
        self.video_source = name.to_string();
        self.stream = None;
    }

    /// Chooses the stream format used from now on and restarts a running stream.
    pub fn set_format(&mut self, format: VideoFormat) -> Result<()> {
        self.preferred_format = Some(format);
        let was_streaming = self.stream.is_some();
        self.stream = None;
        if was_streaming {
            self.ensure_streaming()?;
        }
        Ok(())
    }

    /// Format of the running stream, if any.
    pub fn current_format(&self) -> Option<VideoFormat> {
        self.stream.as_ref().map(PreviewCapture::format)
    }

    /// Formats the camera offers (known once a stream has been opened).
    pub fn available_formats(&self) -> &[VideoFormat] {
        &self.formats
    }

    /// Next frame at the stream's full resolution (not decimated).
    pub fn full_resolution_frame(&mut self) -> Result<Frame> {
        self.ensure_streaming()?;
        let Some(stream) = self.stream.as_mut() else {
            return Err(CameraError::DeviceNotFound);
        };
        stream.request_full_resolution();
        for _ in 0..FULL_FRAME_ATTEMPTS {
            if let Some(frame) = stream.next_frame()? {
                return Ok(frame);
            }
        }
        Err(CameraError::ReadFailed("no frame received".into()))
    }

    pub fn stop_streaming(&mut self) {
        self.stream = None;
    }

    pub fn is_streaming(&self) -> bool {
        self.stream.is_some()
    }

    /// Next preview frame; starts the stream on first use.
    pub fn next_frame(&mut self) -> Result<Option<Frame>> {
        self.ensure_streaming()?;
        match self.stream.as_mut() {
            Some(stream) => stream.next_frame(),
            None => Ok(None),
        }
    }

    /// Builds limits from what the device reports, falling back to the driver defaults.
    fn read_limits(controls: &StandardControls) -> PtzLimits {
        let fallback = PtzLimits::LINK_2;
        // DirectShow reports pan/tilt in whole degrees; core units are 1/3600 degree.
        let axis_range = |axis, default: AngleRange| match controls.ptz(axis) {
            Ok(c) if c.max > c.min => AngleRange::degrees(c.min, c.max),
            _ => default,
        };
        let zoom = match controls.ptz(PtzAxis::Zoom) {
            Ok(c) if c.max > c.min && c.min >= 0 => ZoomRange { min: c.min as u16, max: c.max as u16 },
            _ => fallback.zoom,
        };
        PtzLimits {
            pan: axis_range(PtzAxis::Pan, fallback.pan),
            tilt: axis_range(PtzAxis::Tilt, fallback.tilt),
            zoom,
        }
    }

    fn read_mode_payload(&self) -> Result<Vec<u8>> {
        let node = self.xu_node.ok_or(CameraError::ControlUnavailable("video mode"))?;
        self.xu.read(node, &KNOWN_XU_GUIDS[PRIMARY_XU_GUID_INDEX], SELECTOR_VIDEO_MODE)
    }

    fn current_mode(&self) -> Option<VideoMode> {
        mode_from_payload(&self.read_mode_payload().ok()?)
    }
}

impl CameraDevice for WinCamera {
    fn limits(&self) -> PtzLimits {
        self.limits
    }

    fn supports_pan_tilt(&self) -> bool {
        false
    }

    fn read_state(&mut self) -> Result<CameraState> {
        let value = |axis| self.controls.ptz(axis).map(|c| c.value);
        // The 2C driver returns unrelated raw numbers for pan/tilt; report them as centred.
        Ok(CameraState {
            pan: AngleUnits::ZERO,
            tilt: AngleUnits::ZERO,
            zoom: ZoomUnits::from_raw(value(PtzAxis::Zoom)? as u16),
            mode: self.current_mode(),
        })
    }

    fn set_zoom(&mut self, zoom: ZoomUnits) -> Result<CameraState> {
        let clamped = ZoomUnits::from_raw_clamped(zoom.raw() as i64, self.limits.zoom);
        self.controls.set_ptz(PtzAxis::Zoom, clamped.raw() as i32)?;
        self.read_state()
    }

    fn set_pan_tilt(&mut self, _pan: AngleUnits, _tilt: AngleUnits) -> Result<CameraState> {
        Err(CameraError::ControlUnavailable("pan/tilt (ignored by the Link 2C firmware)"))
    }

    fn set_mode(&mut self, mode: VideoMode) -> Result<CameraState> {
        let node = self.xu_node.ok_or(CameraError::ControlUnavailable("video mode"))?;
        self.ensure_streaming()?;
        let guid = KNOWN_XU_GUIDS[PRIMARY_XU_GUID_INDEX];

        let current = self.xu.read(node, &guid, SELECTOR_VIDEO_MODE)?;
        let payload = payload_with_mode(&current, mode)
            .ok_or_else(|| CameraError::ReadFailed("mode payload too short".into()))?;
        self.xu.set(node, &guid, SELECTOR_VIDEO_MODE, &payload)?;

        // The echo appears once the firmware has applied the mode; poll for it.
        let mut echoed = None;
        for _ in 0..MODE_POLLS {
            sleep(MODE_POLL_INTERVAL);
            echoed = self.current_mode();
            if echoed == Some(mode) {
                break;
            }
        }
        if echoed != Some(mode) {
            return Err(CameraError::WriteFailed(format!(
                "camera did not accept `{}` (echo {:?}); modes only apply while streaming",
                mode.label(),
                echoed.map(VideoMode::label)
            )));
        }
        self.read_state()
    }

    fn image_control(&mut self, control: ImageControl) -> Result<ControlValue> {
        self.controls.image(control)
    }

    fn set_image_control(&mut self, control: ImageControl, value: i32, auto: bool) -> Result<()> {
        self.controls.set_image(control, value, auto)
    }
}

/// Raw access to the extension units for diagnostics (`probe` tool).
pub struct ProbeHandle {
    info: CameraInfo,
    xu: XuClient,
    _filter: IBaseFilter,
}

impl ProbeHandle {
    pub fn open() -> Result<Self> {
        let (info, filter) = open_insta360_filter()?;
        let xu = XuClient::new(&filter)?;
        Ok(Self { info, xu, _filter: filter })
    }

    pub fn info(&self) -> &CameraInfo {
        &self.info
    }

    pub fn xu(&self) -> &XuClient {
        &self.xu
    }
}
