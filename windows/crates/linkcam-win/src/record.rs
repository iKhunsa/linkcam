//! H.264/MP4 recording of preview frames through Media Foundation's sink writer.

use crate::audio::AudioFormat;
use crate::preview::Frame;
use linkcam_core::{CameraError, Result};
use std::path::Path;
use std::time::Instant;
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;

const BITRATE_BITS_PER_SECOND: u32 = 10_000_000;
/// AAC encoder bit rate in bytes per second (192 kbit/s; the MFT accepts 12000/16000/20000/24000).
const AAC_BYTES_PER_SECOND: u32 = 24_000;
const AAC_PROFILE_LEVEL_L2: u32 = 0x29;
const PCM_BITS: u32 = 16;
const NANOS_PER_TICK: u128 = 100;
const TICKS_PER_SECOND: u64 = 10_000_000;

fn fail(what: &str) -> impl Fn(windows::core::Error) -> CameraError + '_ {
    move |e| CameraError::WriteFailed(format!("{what}: {e}"))
}

struct AudioTrack {
    stream: u32,
    format: AudioFormat,
    frames_written: u64,
}

pub struct Recorder {
    writer: IMFSinkWriter,
    stream: u32,
    audio: Option<AudioTrack>,
    width: usize,
    height: usize,
    frame_ticks: u64,
    started: Instant,
    finished: bool,
}

impl Recorder {
    /// Starts a recording of `width`x`height` frames at `fps` into `path` (.mp4),
    /// with an AAC audio track when `audio` is given.
    pub fn create(
        path: &Path,
        width: usize,
        height: usize,
        fps: u32,
        audio: Option<AudioFormat>,
    ) -> Result<Self> {
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
        let audio = match audio {
            Some(format) => Some(add_audio_stream(&writer, format)?),
            None => None,
        };
        unsafe { writer.BeginWriting() }.map_err(fail("BeginWriting"))?;

        Ok(Self {
            writer,
            stream,
            audio,
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

    /// Appends interleaved 16-bit PCM; timestamps follow the sample count.
    pub fn write_audio(&mut self, samples: &[i16]) -> Result<()> {
        let Some(track) = self.audio.as_mut() else { return Ok(()) };
        let channels = track.format.channels as usize;
        if samples.is_empty() || channels == 0 {
            return Ok(());
        }
        let frames = (samples.len() / channels) as u64;
        let bytes: Vec<u8> =
            samples[..frames as usize * channels].iter().flat_map(|s| s.to_le_bytes()).collect();

        let buffer = unsafe { MFCreateMemoryBuffer(bytes.len() as u32) }.map_err(fail("audio buffer"))?;
        let mut data = std::ptr::null_mut();
        unsafe { buffer.Lock(&mut data, None, None) }.map_err(fail("Lock"))?;
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
        let _ = unsafe { buffer.Unlock() };
        unsafe { buffer.SetCurrentLength(bytes.len() as u32) }.map_err(fail("SetCurrentLength"))?;

        let rate = track.format.sample_rate as u64;
        let sample = unsafe { MFCreateSample() }.map_err(fail("audio sample"))?;
        unsafe {
            sample.AddBuffer(&buffer).map_err(fail("AddBuffer"))?;
            sample
                .SetSampleTime((track.frames_written * TICKS_PER_SECOND / rate) as i64)
                .map_err(fail("time"))?;
            sample.SetSampleDuration((frames * TICKS_PER_SECOND / rate) as i64).map_err(fail("duration"))?;
            self.writer.WriteSample(track.stream, &sample).map_err(fail("WriteSample(audio)"))?;
        }
        track.frames_written += frames;
        Ok(())
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

fn add_audio_stream(writer: &IMFSinkWriter, format: AudioFormat) -> Result<AudioTrack> {
    let output = unsafe { MFCreateMediaType() }.map_err(fail("media type"))?;
    let input = unsafe { MFCreateMediaType() }.map_err(fail("media type"))?;
    let block_align = format.channels as u32 * PCM_BITS / 8;
    unsafe {
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(fail("major"))?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC).map_err(fail("subtype"))?;
        output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, PCM_BITS).map_err(fail("bits"))?;
        output.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, format.sample_rate).map_err(fail("rate"))?;
        output.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, format.channels as u32).map_err(fail("channels"))?;
        output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AAC_BYTES_PER_SECOND).map_err(fail("bytes"))?;
        output.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 1).map_err(fail("align"))?;
        output.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0).map_err(fail("payload"))?;
        output
            .SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, AAC_PROFILE_LEVEL_L2)
            .map_err(fail("profile"))?;

        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(fail("major"))?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(fail("subtype"))?;
        input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, PCM_BITS).map_err(fail("bits"))?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, format.sample_rate).map_err(fail("rate"))?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, format.channels as u32).map_err(fail("channels"))?;
        input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align).map_err(fail("align"))?;
        input
            .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, format.sample_rate * block_align)
            .map_err(fail("bytes"))?;
    }
    let stream = unsafe { writer.AddStream(&output) }.map_err(fail("AddStream(audio)"))?;
    unsafe { writer.SetInputMediaType(stream, &input, None) }.map_err(fail("SetInputMediaType(audio)"))?;
    Ok(AudioTrack { stream, format, frames_written: 0 })
}

/// What a finished media file contains: `(has_video, has_audio)`.
pub fn describe_file(path: &Path) -> Result<(bool, bool)> {
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(fail("MFStartup"))?;
    let reader = unsafe { MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None) };
    let result = reader.map_err(fail("open file")).map(|reader| {
        let (mut video, mut audio) = (false, false);
        for index in 0..8u32 {
            let Ok(kind) = (unsafe { reader.GetNativeMediaType(index, 0) }) else { break };
            match unsafe { kind.GetGUID(&MF_MT_MAJOR_TYPE) } {
                Ok(major) if major == MFMediaType_Video => video = true,
                Ok(major) if major == MFMediaType_Audio => audio = true,
                _ => {}
            }
        }
        (video, audio)
    });
    let _ = unsafe { MFShutdown() };
    result
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes synthetic video + PCM and checks the MP4 holds both tracks.
    #[test]
    fn recording_with_audio_produces_video_and_audio_tracks() {
        let path = std::env::temp_dir().join("linkcam-record-test.mp4");
        let _ = std::fs::remove_file(&path);
        let format = AudioFormat { sample_rate: 48_000, channels: 2 };
        let mut recorder = Recorder::create(&path, 640, 360, 30, Some(format)).unwrap();

        let frame = Frame { width: 640, height: 360, rgba: vec![90; 640 * 360 * 4] };
        for step in 0..60 {
            recorder.write_frame(&frame).unwrap();
            // 1/30 s of a 440 Hz tone, interleaved stereo.
            let chunk: Vec<i16> = (0..1600)
                .flat_map(|i| {
                    let t = (step * 1600 + i) as f32 / 48_000.0;
                    let v = ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16;
                    [v, v]
                })
                .collect();
            recorder.write_audio(&chunk).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        recorder.finish().unwrap();

        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 10_000, "file is {size} bytes");
        assert_eq!(describe_file(&path).unwrap(), (true, true));
    }
}
