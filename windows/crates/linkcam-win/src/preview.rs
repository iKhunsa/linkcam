//! Video capture through Media Foundation's source reader.
//!
//! `MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING` makes the reader convert
//! whatever the camera sends (MJPEG/NV12/YUY2) to RGB32 for us, so no
//! decoder is needed in this crate.
//!
//! Frames are decimated to at most `MAX_PREVIEW_WIDTH` pixels wide so a 4K
//! stream stays cheap to display; a full-resolution frame can be requested
//! once (snapshots).

use linkcam_core::{CameraError, Result, VideoFormat};
use windows::core::PWSTR;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;

/// One captured frame in tightly packed RGBA, top row first.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// Nearest-neighbour downscale used for the gesture engine's small input.
    pub fn downscaled(&self, target_width: usize) -> Frame {
        let target_width = target_width.clamp(1, self.width);
        let target_height = (self.height * target_width / self.width).max(1);
        let mut rgba = Vec::with_capacity(target_width * target_height * 4);
        for y in 0..target_height {
            let source_y = y * self.height / target_height;
            for x in 0..target_width {
                let source_x = x * self.width / target_width;
                let at = (source_y * self.width + source_x) * 4;
                rgba.extend_from_slice(&self.rgba[at..at + 4]);
            }
        }
        Frame { width: target_width, height: target_height, rgba }
    }
}

/// Widest preview frame handed to the UI; larger streams are decimated.
const MAX_PREVIEW_WIDTH: usize = 1280;
/// Size used when no format has been chosen.
const DEFAULT_WIDTH: u32 = 1280;

fn fail(what: &str) -> impl Fn(windows::core::Error) -> CameraError + '_ {
    move |e| CameraError::ReadFailed(format!("{what}: {e}"))
}

/// Keeps Media Foundation started for as long as it lives.
struct MediaFoundation;

impl MediaFoundation {
    fn start() -> Result<Self> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(fail("MFStartup"))?;
        Ok(Self)
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

pub struct PreviewCapture {
    reader: IMFSourceReader,
    format: VideoFormat,
    available: Vec<VideoFormat>,
    full_resolution_once: bool,
    // Field order matters: the reader must drop before Media Foundation shuts down.
    _source: IMFMediaSource,
    _mf: MediaFoundation,
}

impl PreviewCapture {
    /// Opens the first video source whose friendly name contains `name_part`,
    /// using the native mode closest to `wanted` (about 720p when `None`).
    pub fn open(name_part: &str, wanted: Option<VideoFormat>) -> Result<Self> {
        let mf = MediaFoundation::start()?;
        let source = open_source(name_part, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;

        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 2) }.map_err(fail("MFCreateAttributes"))?;
        let attributes: IMFAttributes = attributes.ok_or(CameraError::DeviceNotFound)?;
        unsafe { attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1) }
            .map_err(fail("reader attrs"))?;
        // Software decoding: hardware MJPEG decoders are a shared, limited resource
        // and fail with "lack of hardware resources" when other apps hold them.
        unsafe { attributes.SetUINT32(&MF_SOURCE_READER_DISABLE_DXVA, 1) }.map_err(fail("reader attrs"))?;
        let reader = unsafe { MFCreateSourceReaderFromMediaSource(&source, &attributes) }
            .map_err(fail("MFCreateSourceReader"))?;

        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let natives = native_formats(&reader, stream);
        let available = VideoFormat::sorted_descending(natives.iter().map(|(f, _)| *f).collect());
        let target = wanted.unwrap_or(VideoFormat::new(DEFAULT_WIDTH, DEFAULT_WIDTH * 9 / 16, 30));
        let format = VideoFormat::closest(&available, target)
            .ok_or(CameraError::ControlUnavailable("video format"))?;

        select_native_type(&reader, stream, &natives, format)?;
        set_rgb32_output(&reader, stream, format)?;
        Ok(Self { reader, format, available, full_resolution_once: false, _source: source, _mf: mf })
    }

    /// Format the stream is actually running at.
    pub fn format(&self) -> VideoFormat {
        self.format
    }

    /// Every distinct native format of the camera, largest first.
    pub fn available_formats(&self) -> &[VideoFormat] {
        &self.available
    }

    /// The next frame is delivered without decimation (for snapshots).
    pub fn request_full_resolution(&mut self) {
        self.full_resolution_once = true;
    }

    /// Blocks until the next frame arrives (or the stream hiccups: returns `None`).
    pub fn next_frame(&mut self) -> Result<Option<Frame>> {
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        unsafe { self.reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample)) }
            .map_err(fail("ReadSample"))?;
        let Some(sample) = sample else { return Ok(None) };

        let buffer =
            unsafe { sample.ConvertToContiguousBuffer() }.map_err(fail("ConvertToContiguousBuffer"))?;
        let (mut data, mut max_len, mut cur_len) = (std::ptr::null_mut(), 0u32, 0u32);
        unsafe { buffer.Lock(&mut data, Some(&mut max_len), Some(&mut cur_len)) }.map_err(fail("Lock"))?;
        let bytes = unsafe { std::slice::from_raw_parts(data, cur_len as usize) };

        let width = self.format.width as usize;
        let height = self.format.height as usize;
        let step = if std::mem::take(&mut self.full_resolution_once) { 1 } else { decimation_step(width) };
        let frame = bgrx_to_rgba(bytes, width, height, step);
        let _ = unsafe { buffer.Unlock() };
        Ok(frame)
    }
}

/// Smallest integer step that brings `width` to at most `MAX_PREVIEW_WIDTH`.
fn decimation_step(width: usize) -> usize {
    width.div_ceil(MAX_PREVIEW_WIDTH).max(1)
}

/// Converts top-down BGRX to RGBA, keeping every `step`-th pixel in both axes.
fn bgrx_to_rgba(bytes: &[u8], width: usize, height: usize, step: usize) -> Option<Frame> {
    let stride = width * 4;
    if bytes.len() < stride * height {
        return None;
    }
    let (out_w, out_h) = (width / step, height / step);
    let mut rgba = Vec::with_capacity(out_w * out_h * 4);
    for y in 0..out_h {
        let row = &bytes[y * step * stride..(y * step + 1) * stride];
        for x in 0..out_w {
            let p = &row[x * step * 4..x * step * 4 + 4];
            rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    Some(Frame { width: out_w, height: out_h, rgba })
}

/// Friendly names of every capture device of `source_type`.
pub(crate) fn list_source_names(source_type: &windows::core::GUID) -> Vec<String> {
    let mut attributes = None;
    if unsafe { MFCreateAttributes(&mut attributes, 1) }.is_err() {
        return Vec::new();
    }
    let Some(attributes): Option<IMFAttributes> = attributes else { return Vec::new() };
    if unsafe { attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, source_type) }.is_err() {
        return Vec::new();
    }
    let mut devices: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    if unsafe { MFEnumDeviceSources(&attributes, &mut devices, &mut count) }.is_err() {
        return Vec::new();
    }
    let activates = unsafe { std::slice::from_raw_parts(devices, count as usize) };
    let names = activates.iter().flatten().map(friendly_name).collect();
    unsafe { CoTaskMemFree(Some(devices.cast())) };
    names
}

/// Opens the first capture device of `source_type` (video or audio) whose name matches.
pub(crate) fn open_source(name_part: &str, source_type: &windows::core::GUID) -> Result<IMFMediaSource> {
    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 1) }.map_err(fail("MFCreateAttributes"))?;
    let attributes: IMFAttributes = attributes.ok_or(CameraError::DeviceNotFound)?;
    unsafe { attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, source_type) }
        .map_err(fail("source type"))?;

    let mut devices: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    unsafe { MFEnumDeviceSources(&attributes, &mut devices, &mut count) }
        .map_err(fail("MFEnumDeviceSources"))?;
    let activates = unsafe { std::slice::from_raw_parts(devices, count as usize) };

    let wanted = name_part.to_ascii_lowercase();
    let mut chosen = None;
    for activate in activates.iter().flatten() {
        if friendly_name(activate).to_ascii_lowercase().contains(&wanted) {
            chosen = unsafe { activate.ActivateObject::<IMFMediaSource>() }.ok();
            break;
        }
    }
    unsafe { CoTaskMemFree(Some(devices.cast())) };
    chosen.ok_or(CameraError::DeviceNotFound)
}

fn friendly_name(activate: &IMFActivate) -> String {
    let mut buffer = PWSTR::null();
    let mut length = 0u32;
    let result = unsafe {
        activate.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut buffer, &mut length)
    };
    if result.is_err() || buffer.is_null() {
        return String::new();
    }
    let name = unsafe { buffer.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(buffer.0.cast())) };
    name
}

/// Every native media type as `(format, native index)`.
fn native_formats(reader: &IMFSourceReader, stream: u32) -> Vec<(VideoFormat, u32)> {
    let mut formats = Vec::new();
    for index in 0.. {
        let Ok(native) = (unsafe { reader.GetNativeMediaType(stream, index) }) else { break };
        let size = unsafe { native.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap_or(0);
        let rate = unsafe { native.GetUINT64(&MF_MT_FRAME_RATE) }.unwrap_or(0);
        let (width, height) = ((size >> 32) as u32, (size & 0xffff_ffff) as u32);
        let (numerator, denominator) = ((rate >> 32) as u32, (rate & 0xffff_ffff) as u32);
        if width > 0 && height > 0 && denominator > 0 {
            let fps = (numerator as f64 / denominator as f64).round() as u32;
            formats.push((VideoFormat::new(width, height, fps), index));
        }
    }
    formats
}

fn select_native_type(
    reader: &IMFSourceReader,
    stream: u32,
    natives: &[(VideoFormat, u32)],
    format: VideoFormat,
) -> Result<()> {
    // Several subtypes (MJPEG/NV12/YUY2) can share a size; take the first that the reader accepts.
    for (_, index) in natives.iter().filter(|(f, _)| *f == format) {
        let Ok(native) = (unsafe { reader.GetNativeMediaType(stream, *index) }) else { continue };
        if unsafe { reader.SetCurrentMediaType(stream, None, &native) }.is_ok() {
            return Ok(());
        }
    }
    Err(CameraError::ControlUnavailable("video format"))
}

fn set_rgb32_output(reader: &IMFSourceReader, stream: u32, format: VideoFormat) -> Result<()> {
    let output = unsafe { MFCreateMediaType() }.map_err(fail("MFCreateMediaType"))?;
    let size = ((format.width as u64) << 32) | format.height as u64;
    unsafe {
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(fail("major"))?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(fail("subtype"))?;
        output.SetUINT64(&MF_MT_FRAME_SIZE, size).map_err(fail("size"))?;
        reader.SetCurrentMediaType(stream, None, &output).map_err(fail("SetCurrentMediaType(RGB32)"))?;
    }
    Ok(())
}

/// Writes an RGBA frame as PNG (used by the snapshot button and diagnostics).
pub fn save_png(frame: &Frame, path: &std::path::Path) -> Result<()> {
    let file = std::fs::File::create(path).map_err(|e| CameraError::WriteFailed(e.to_string()))?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), frame.width as u32, frame.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| CameraError::WriteFailed(e.to_string()))?;
    writer.write_image_data(&frame.rgba).map_err(|e| CameraError::WriteFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgrx_is_swizzled_to_rgba() {
        let bytes = [10, 20, 30, 0, 40, 50, 60, 0];
        let frame = bgrx_to_rgba(&bytes, 2, 1, 1).unwrap();
        assert_eq!(frame.rgba, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }

    #[test]
    fn decimation_keeps_every_nth_pixel() {
        // 4x2 image, step 2 -> 2x1 taking pixels (0,0) and (2,0).
        let mut bytes = vec![0u8; 4 * 2 * 4];
        bytes[0..4].copy_from_slice(&[1, 2, 3, 0]);
        bytes[8..12].copy_from_slice(&[7, 8, 9, 0]);
        let frame = bgrx_to_rgba(&bytes, 4, 2, 2).unwrap();
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(&frame.rgba[..4], &[3, 2, 1, 255]);
        assert_eq!(&frame.rgba[4..], &[9, 8, 7, 255]);
    }

    #[test]
    fn decimation_step_caps_the_width() {
        assert_eq!(decimation_step(1280), 1);
        assert_eq!(decimation_step(1920), 2);
        assert_eq!(decimation_step(3840), 3);
    }

    #[test]
    fn downscale_keeps_aspect_ratio() {
        let frame = Frame { width: 8, height: 4, rgba: vec![0; 8 * 4 * 4] };
        let small = frame.downscaled(4);
        assert_eq!((small.width, small.height), (4, 2));
        assert_eq!(small.rgba.len(), 4 * 2 * 4);
    }
}
