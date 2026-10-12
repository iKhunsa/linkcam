//! Microphone capture through a Media Foundation source reader on its own
//! thread; the app drains the PCM it produces while recording.

use crate::preview::{list_source_names, open_source};
use crate::ComGuard;
use linkcam_core::{CameraError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use windows::Win32::Media::MediaFoundation::*;

/// PCM format delivered by `MicCapture` (always signed 16-bit, interleaved).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Name fragment of the camera's microphone endpoint.
const CAMERA_MIC_NAME: &str = "Insta360";
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const BITS_PER_SAMPLE: u32 = 16;
/// How long `start` waits for the capture thread to report success or failure.
const START_TIMEOUT: Duration = Duration::from_secs(5);

fn fail(what: &str) -> impl Fn(windows::core::Error) -> CameraError + '_ {
    move |e| CameraError::ReadFailed(format!("{what}: {e}"))
}

pub struct MicCapture {
    format: AudioFormat,
    chunks: Receiver<Vec<i16>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MicCapture {
    /// Starts capturing from the microphone whose name contains `name_part`.
    /// An empty `name_part` means automatic: the camera's own microphone when it
    /// has one, otherwise the first available.
    pub fn start(name_part: &str) -> Result<Self> {
        let (chunk_tx, chunk_rx) = channel();
        let (ready_tx, ready_rx) = sync_channel::<Result<AudioFormat>>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_stop, name) = (stop.clone(), name_part.to_string());
        let thread = std::thread::Builder::new()
            .name("microphone".into())
            .spawn(move || {
                let _com = ComGuard::init();
                match open_reader(&name) {
                    Ok((reader, _mf)) => {
                        let _ =
                            ready_tx.send(Ok(AudioFormat { sample_rate: SAMPLE_RATE, channels: CHANNELS }));
                        pump(&reader, &chunk_tx, &thread_stop);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            })
            .map_err(|e| CameraError::ReadFailed(e.to_string()))?;

        let format = ready_rx
            .recv_timeout(START_TIMEOUT)
            .map_err(|_| CameraError::ReadFailed("the microphone did not start in time".into()))??;
        Ok(Self { format, chunks: chunk_rx, stop, thread: Some(thread) })
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// Everything captured since the last call, interleaved.
    pub fn drain(&self) -> Vec<i16> {
        let mut samples = Vec::new();
        while let Ok(chunk) = self.chunks.try_recv() {
            samples.extend_from_slice(&chunk);
        }
        samples
    }
}

impl Drop for MicCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Keeps Media Foundation started while the reader lives.
struct MfSession;

impl Drop for MfSession {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

/// Names of the microphones Media Foundation can capture from.
pub fn list_microphones() -> Vec<String> {
    let _com = ComGuard::init();
    if unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.is_err() {
        return Vec::new();
    }
    let names = list_source_names(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_GUID);
    let _ = unsafe { MFShutdown() };
    names
}

fn open_reader(name_part: &str) -> Result<(IMFSourceReader, MfSession)> {
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.map_err(fail("MFStartup"))?;
    let session = MfSession;
    let source = if name_part.is_empty() {
        // Automatic: prefer the camera's own microphone, else anything that works.
        open_source(CAMERA_MIC_NAME, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_GUID)
            .or_else(|_| open_source("", &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_GUID))?
    } else {
        open_source(name_part, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_AUDCAP_GUID)?
    };
    let reader =
        unsafe { MFCreateSourceReaderFromMediaSource(&source, None) }.map_err(fail("source reader"))?;

    let pcm = unsafe { MFCreateMediaType() }.map_err(fail("media type"))?;
    let block_align = CHANNELS as u32 * BITS_PER_SAMPLE / 8;
    unsafe {
        pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(fail("major"))?;
        pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(fail("subtype"))?;
        pcm.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32).map_err(fail("channels"))?;
        pcm.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, SAMPLE_RATE).map_err(fail("rate"))?;
        pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, BITS_PER_SAMPLE).map_err(fail("bits"))?;
        pcm.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align).map_err(fail("align"))?;
        pcm.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, SAMPLE_RATE * block_align).map_err(fail("bytes"))?;
        reader
            .SetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, None, &pcm)
            .map_err(fail("SetCurrentMediaType(PCM)"))?;
    }
    Ok((reader, session))
}

fn pump(reader: &IMFSourceReader, out: &std::sync::mpsc::Sender<Vec<i16>>, stop: &AtomicBool) {
    let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    while !stop.load(Ordering::Relaxed) {
        let (mut flags, mut sample) = (0u32, None::<IMFSample>);
        if unsafe { reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample)) }.is_err() {
            break;
        }
        let Some(sample) = sample else { continue };
        let Ok(buffer) = (unsafe { sample.ConvertToContiguousBuffer() }) else { continue };
        let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
        if unsafe { buffer.Lock(&mut data, None, Some(&mut length)) }.is_err() {
            continue;
        }
        let bytes = unsafe { std::slice::from_raw_parts(data, length as usize) };
        let samples: Vec<i16> = bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        let _ = unsafe { buffer.Unlock() };
        if out.send(samples).is_err() {
            break;
        }
    }
}
