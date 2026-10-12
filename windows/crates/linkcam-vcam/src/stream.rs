//! The single video stream. Sample requests are answered by a paced worker
//! thread so consumers receive frames at the nominal rate.

use linkcam_core::vcam::{bgra_to_nv12, FPS, FRAME_BYTES};
use linkcam_win::vcam_channel::FrameReader;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::core::{implement, ComObject, ComObjectInner, IUnknown, Ref, GUID, HRESULT};
use windows::Win32::Foundation::E_UNEXPECTED;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

/// Sample-request token. Media Foundation objects are free-threaded, so moving
/// the reference to the pacing thread is sound.
struct Token(Option<IUnknown>);

unsafe impl Send for Token {}

/// What the pacing thread owns. The Media Foundation objects are free-threaded.
struct PacingWork {
    requests: Receiver<Token>,
    queue: IMFMediaEventQueue,
    descriptor: IMFStreamDescriptor,
}

unsafe impl Send for PacingWork {}

struct Control {
    sender: Option<Sender<Token>>,
    worker: Option<JoinHandle<()>>,
    state: MF_STREAM_STATE,
}

#[implement(IMFMediaStream2)]
pub struct MediaStream {
    source: IMFMediaSource,
    queue: IMFMediaEventQueue,
    descriptor: IMFStreamDescriptor,
    control: Mutex<Control>,
}

impl MediaStream {
    pub fn create(
        source: IMFMediaSource,
        descriptor: IMFStreamDescriptor,
    ) -> windows::core::Result<ComObject<Self>> {
        let queue = unsafe { MFCreateEventQueue()? };
        let stream = MediaStream {
            source,
            queue,
            descriptor,
            control: Mutex::new(Control { sender: None, worker: None, state: MF_STREAM_STATE_STOPPED }),
        };
        Ok(stream.into_object())
    }

    fn locked(&self) -> windows::core::Result<std::sync::MutexGuard<'_, Control>> {
        self.control.lock().map_err(|_| E_UNEXPECTED.into())
    }

    /// Starts answering sample requests.
    pub fn start(&self) -> windows::core::Result<()> {
        let mut control = self.locked()?;
        if control.sender.is_none() {
            let (sender, receiver) = channel();
            let work = PacingWork {
                requests: receiver,
                queue: self.queue.clone(),
                descriptor: self.descriptor.clone(),
            };
            control.worker = std::thread::Builder::new()
                .name("vcam-stream".into())
                .spawn(move || {
                    let work = work;
                    pace_samples(work.requests, work.queue, work.descriptor)
                })
                .ok();
            control.sender = Some(sender);
        }
        control.state = MF_STREAM_STATE_RUNNING;
        unsafe {
            self.queue.QueueEventParamVar(
                MEStreamStarted.0 as u32,
                &GUID::zeroed(),
                HRESULT(0),
                &PROPVARIANT::default(),
            )
        }
    }

    pub fn stop(&self) -> windows::core::Result<()> {
        let mut control = self.locked()?;
        control.state = MF_STREAM_STATE_STOPPED;
        control.sender = None; // lets the worker finish
        unsafe {
            self.queue.QueueEventParamVar(
                MEStreamStopped.0 as u32,
                &GUID::zeroed(),
                HRESULT(0),
                &PROPVARIANT::default(),
            )
        }
    }

    pub fn shutdown(&self) {
        if let Ok(mut control) = self.control.lock() {
            control.sender = None;
            if let Some(worker) = control.worker.take() {
                drop(control);
                let _ = worker.join();
            }
        }
        let _ = unsafe { self.queue.Shutdown() };
    }
}

impl IMFMediaEventGenerator_Impl for MediaStream_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> windows::core::Result<IMFMediaEvent> {
        unsafe { self.queue.GetEvent(flags.0) }
    }

    fn BeginGetEvent(
        &self,
        callback: Ref<IMFAsyncCallback>,
        state: Ref<IUnknown>,
    ) -> windows::core::Result<()> {
        unsafe { self.queue.BeginGetEvent(callback.as_ref(), state.as_ref()) }
    }

    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> windows::core::Result<IMFMediaEvent> {
        unsafe { self.queue.EndGetEvent(result.as_ref()) }
    }

    fn QueueEvent(
        &self,
        kind: u32,
        extended_type: *const GUID,
        status: HRESULT,
        value: *const PROPVARIANT,
    ) -> windows::core::Result<()> {
        unsafe { self.queue.QueueEventParamVar(kind, extended_type, status, value) }
    }
}

impl IMFMediaStream_Impl for MediaStream_Impl {
    fn GetMediaSource(&self) -> windows::core::Result<IMFMediaSource> {
        Ok(self.source.clone())
    }

    fn GetStreamDescriptor(&self) -> windows::core::Result<IMFStreamDescriptor> {
        Ok(self.descriptor.clone())
    }

    fn RequestSample(&self, token: Ref<IUnknown>) -> windows::core::Result<()> {
        let control = self.locked()?;
        match control.sender.as_ref() {
            Some(sender) => sender.send(Token(token.as_ref().cloned())).map_err(|_| E_UNEXPECTED.into()),
            None => Err(MF_E_MEDIA_SOURCE_WRONGSTATE.into()),
        }
    }
}

impl IMFMediaStream2_Impl for MediaStream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> windows::core::Result<()> {
        self.locked()?.state = value;
        Ok(())
    }

    fn GetStreamState(&self) -> windows::core::Result<MF_STREAM_STATE> {
        Ok(self.locked()?.state)
    }
}

/// Answers each request at the nominal frame rate with the latest published frame.
fn pace_samples(requests: Receiver<Token>, queue: IMFMediaEventQueue, descriptor: IMFStreamDescriptor) {
    let interval = Duration::from_nanos(1_000_000_000 / FPS as u64);
    let mut next = Instant::now();
    let mut reader: Option<FrameReader> = None;
    let mut bgra = vec![0u8; FRAME_BYTES];
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel[3] = 255; // opaque black until the app publishes something
    }

    while let Ok(token) = requests.recv() {
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        next = next.max(now) + interval;

        if reader.is_none() {
            reader = FrameReader::open().ok();
        }
        if let Some(reader) = reader.as_mut() {
            reader.latest(&mut bgra);
        }
        if let Err(_error) = deliver(&queue, &descriptor, &bgra, token) {
            // The consumer went away; keep serving until the channel closes.
        }
    }
}

fn deliver(
    queue: &IMFMediaEventQueue,
    descriptor: &IMFStreamDescriptor,
    bgra: &[u8],
    token: Token,
) -> windows::core::Result<()> {
    unsafe {
        let current = descriptor.GetMediaTypeHandler()?.GetCurrentMediaType()?;
        let subtype = current.GetGUID(&MF_MT_SUBTYPE)?;
        let data = if subtype == MFVideoFormat_NV12 {
            bgra_to_nv12(bgra, linkcam_core::vcam::WIDTH, linkcam_core::vcam::HEIGHT)
        } else {
            bgra.to_vec()
        };

        let buffer = MFCreateMemoryBuffer(data.len() as u32)?;
        let mut destination = std::ptr::null_mut();
        buffer.Lock(&mut destination, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), destination, data.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(data.len() as u32)?;

        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(MFGetSystemTime())?;
        sample.SetSampleDuration(10_000_000 / FPS as i64)?;
        if let Some(token) = token.0 {
            sample.SetUnknown(&MFSampleExtension_Token, &token)?;
        }
        queue.QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), HRESULT(0), &sample)
    }
}
