//! H.264/MP4 recording of preview frames through Media Foundation's sink writer.

use crate::preview::Frame;
use linkcam_core::{CameraError, Result};
use std::path::Path;
use std::time::Instant;
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;

const BITRATE_BITS_PER_SECOND: u32 = 10_000_000;
const NANOS_PER_TICK: u128 = 100;
const TICKS_PER_SECOND: u64 = 10_000_000;

fn fail(what: &str) -> impl Fn(windows::core::Error) -> CameraError + '_ {
    move |e| CameraError::WriteFailed(format!("{what}: {e}"))
}

pub struct Recorder {
    writer: IMFSinkWriter,
    stream: u32,
    width: usize,
    height: usize,
    frame_ticks: u64,
    started: Instant,
    finished: bool,
}

impl Recorder {
    /// Starts a recording of `width`x`height` frames at `fps` into `path` (.mp4).
    pub fn create(path: &Path, width: usize, height: usize, fps: u32) -> Result<Self> {
        // H.264 needs even dimensions.
        let (width, height) = (width & !1, height & !1);
        let fps = fps.max(1);
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(fail("MFStartup"))?;

        let url = HSTRING::from(path.as_os_str());
        let writer = unsafe { MFCreateSinkWriterFromURL(&url, None, None) }.map_err(fail("sink writer"))?;

        let size = ((width as u64) << 32) | height as u64;
        let rate = ((fps as u64) << 32) | 1;
        let aspect = (1u64 << 32) | 1;

        let output = unsafe { MFCreateMediaType() }.map_err(fail("media type"))?;
        let input = unsafe { MFCreateMediaType() }.map_err(fail("media type"))?;
        unsafe {
            output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(fail("major"))?;
            output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(fail("subtype"))?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, BITRATE_BITS_PER_SECOND).map_err(fail("bitrate"))?;
            output
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(fail("interlace"))?;
            output.SetUINT64(&MF_MT_FRAME_SIZE, size).map_err(fail("size"))?;
            output.SetUINT64(&MF_MT_FRAME_RATE, rate).map_err(fail("rate"))?;
            output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, aspect).map_err(fail("aspect"))?;

            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(fail("major"))?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(fail("subtype"))?;
            input
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(fail("interlace"))?;
            input.SetUINT64(&MF_MT_FRAME_SIZE, size).map_err(fail("size"))?;
            input.SetUINT64(&MF_MT_FRAME_RATE, rate).map_err(fail("rate"))?;
            input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, aspect).map_err(fail("aspect"))?;
            // A negative stride marks bottom-up RGB; our rows are top-down.
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, (width * 4) as u32).map_err(fail("stride"))?;
        }
        let stream = unsafe { writer.AddStream(&output) }.map_err(fail("AddStream"))?;
        unsafe { writer.SetInputMediaType(stream, &input, None) }.map_err(fail("SetInputMediaType"))?;
        unsafe { writer.BeginWriting() }.map_err(fail("BeginWriting"))?;

        Ok(Self {
            writer,
            stream,
            width,
            height,
            frame_ticks: TICKS_PER_SECOND / fps as u64,
            started: Instant::now(),
            finished: false,
        })
    }

    pub fn elapsed_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Appends a frame stamped with the wall-clock time since the start.
    pub fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        if frame.width < self.width || frame.height < self.height {
            return Err(CameraError::InvalidArgument("frame smaller than the recording size".into()));
        }
        let length = self.width * self.height * 4;
        let buffer = unsafe { MFCreateMemoryBuffer(length as u32) }.map_err(fail("buffer"))?;
        let mut data = std::ptr::null_mut();
        unsafe { buffer.Lock(&mut data, None, None) }.map_err(fail("Lock"))?;
        let destination = unsafe { std::slice::from_raw_parts_mut(data, length) };
        for y in 0..self.height {
            let source = &frame.rgba[y * frame.width * 4..y * frame.width * 4 + self.width * 4];
            let target = &mut destination[y * self.width * 4..(y + 1) * self.width * 4];
            let (source_pixels, _) = source.as_chunks::<4>();
            let (target_pixels, _) = target.as_chunks_mut::<4>();
            for (s, t) in source_pixels.iter().zip(target_pixels.iter_mut()) {
                *t = [s[2], s[1], s[0], 255];
            }
        }
        let _ = unsafe { buffer.Unlock() };
        unsafe { buffer.SetCurrentLength(length as u32) }.map_err(fail("SetCurrentLength"))?;

        let sample = unsafe { MFCreateSample() }.map_err(fail("sample"))?;
        let timestamp = (self.started.elapsed().as_nanos() / NANOS_PER_TICK) as i64;
        unsafe {
            sample.AddBuffer(&buffer).map_err(fail("AddBuffer"))?;
            sample.SetSampleTime(timestamp).map_err(fail("time"))?;
            sample.SetSampleDuration(self.frame_ticks as i64).map_err(fail("duration"))?;
            self.writer.WriteSample(self.stream, &sample).map_err(fail("WriteSample"))
        }
    }

    /// Finalises the file. Dropping without calling this also finalises it.
    pub fn finish(mut self) -> Result<()> {
        self.finalize()
    }

    fn finalize(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let result = unsafe { self.writer.Finalize() }.map_err(fail("Finalize"));
        let _ = unsafe { MFShutdown() };
        result
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}
