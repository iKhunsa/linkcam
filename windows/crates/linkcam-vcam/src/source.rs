//! The media source: one live video stream (NV12 or RGB32, 1280x720@30).

use crate::stream::MediaStream;
use linkcam_core::vcam::{FPS, HEIGHT, WIDTH};
use std::ffi::c_void;
use std::sync::Mutex;
use windows::core::{implement, ComObject, ComObjectInner, IUnknown, Interface, Ref, GUID, HRESULT};
use windows::Win32::Foundation::{E_NOTIMPL, E_UNEXPECTED};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, IKsControl_Impl, KSIDENTIFIER, PINNAME_VIDEO_CAPTURE,
};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

/// HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND): "this property set is not supported".
const SET_NOT_FOUND: HRESULT = HRESULT(0x8007_0492_u32 as i32);

fn pack(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn media_type(subtype: GUID) -> windows::core::Result<IMFMediaType> {
    let nv12 = subtype == MFVideoFormat_NV12;
    let (width, height) = (WIDTH as u32, HEIGHT as u32);
    let (sample_size, stride) =
        if nv12 { (width * height * 3 / 2, width) } else { (width * height * 4, width * 4) };
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(FPS, 1))?;
        t.SetUINT64(&MF_MT_FRAME_RATE_RANGE_MIN, pack(FPS, 1))?;
        t.SetUINT64(&MF_MT_FRAME_RATE_RANGE_MAX, pack(FPS, 1))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        t.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        t.SetUINT32(&MF_MT_SAMPLE_SIZE, sample_size)?;
        t.SetUINT32(&MF_MT_DEFAULT_STRIDE, stride)?;
        let range = if nv12 { MFNominalRange_16_235 } else { MFNominalRange_0_255 };
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, range.0 as u32)?;
        Ok(t)
    }
}

fn stream_descriptor() -> windows::core::Result<IMFStreamDescriptor> {
    let types = [media_type(MFVideoFormat_NV12)?, media_type(MFVideoFormat_RGB32)?];
    unsafe {
        let descriptor = MFCreateStreamDescriptor(0, &[Some(types[0].clone()), Some(types[1].clone())])?;
        descriptor.GetMediaTypeHandler()?.SetCurrentMediaType(&types[0])?;
        descriptor.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
        descriptor.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
        descriptor.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
        descriptor
            .SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;
        Ok(descriptor)
    }
}

#[derive(Default)]
struct State {
    stream: Option<ComObject<MediaStream>>,
    /// Interface of this very object, handed to the stream; cleared on shutdown.
    me: Option<IMFMediaSource>,
    shutdown: bool,
}

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl)]
pub struct MediaSource {
    queue: IMFMediaEventQueue,
    attributes: IMFAttributes,
    descriptor: IMFStreamDescriptor,
    state: Mutex<State>,
}

impl MediaSource {
    pub fn create() -> windows::core::Result<IMFMediaSourceEx> {
        unsafe {
            let queue = MFCreateEventQueue()?;
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
            attributes.SetString(
                &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                windows::core::w!("LinkCam Virtual Camera"),
            )?;
            let object = MediaSource {
                queue,
                attributes,
                descriptor: stream_descriptor()?,
                state: Mutex::new(State::default()),
            }
            .into_object();
            let source: IMFMediaSourceEx = object.to_interface();
            if let Ok(mut state) = object.state.lock() {
                state.me = Some(source.cast()?);
            }
            Ok(source)
        }
    }

    fn locked(&self) -> windows::core::Result<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| E_UNEXPECTED.into())
    }
}

impl IMFMediaEventGenerator_Impl for MediaSource_Impl {
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

impl IMFMediaSource_Impl for MediaSource_Impl {
    fn GetCharacteristics(&self) -> windows::core::Result<u32> {
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> windows::core::Result<IMFPresentationDescriptor> {
        unsafe {
            let presentation = MFCreatePresentationDescriptor(Some(&[Some(self.descriptor.clone())]))?;
            presentation.SelectStream(0)?;
            Ok(presentation)
        }
    }

    fn Start(
        &self,
        _presentation: Ref<IMFPresentationDescriptor>,
        _time_format: *const GUID,
        start_position: *const PROPVARIANT,
    ) -> windows::core::Result<()> {
        let mut state = self.locked()?;
        if state.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        let first_time = state.stream.is_none();
        if first_time {
            let source = state.me.clone().ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
            state.stream = Some(MediaStream::create(source, self.descriptor.clone())?);
        }
        let Some(stream) = state.stream.as_ref() else { return Err(E_UNEXPECTED.into()) };
        let stream_interface: IMFMediaStream2 = stream.to_interface();
        unsafe {
            let kind = if first_time { MENewStream } else { MEUpdatedStream };
            self.queue.QueueEventParamUnk(kind.0 as u32, &GUID::zeroed(), HRESULT(0), &stream_interface)?;
            self.queue.QueueEventParamVar(
                MESourceStarted.0 as u32,
                &GUID::zeroed(),
                HRESULT(0),
                start_position,
            )?;
        }
        stream.start()
    }

    fn Stop(&self) -> windows::core::Result<()> {
        let state = self.locked()?;
        if let Some(stream) = state.stream.as_ref() {
            stream.stop()?;
        }
        unsafe {
            self.queue.QueueEventParamVar(
                MESourceStopped.0 as u32,
                &GUID::zeroed(),
                HRESULT(0),
                &PROPVARIANT::default(),
            )
        }
    }

    fn Pause(&self) -> windows::core::Result<()> {
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }

    fn Shutdown(&self) -> windows::core::Result<()> {
        let mut state = self.locked()?;
        state.shutdown = true;
        if let Some(stream) = state.stream.take() {
            stream.shutdown();
        }
        state.me = None;
        unsafe { self.queue.Shutdown() }
    }
}

impl IMFMediaSourceEx_Impl for MediaSource_Impl {
    fn GetSourceAttributes(&self) -> windows::core::Result<IMFAttributes> {
        Ok(self.attributes.clone())
    }

    fn GetStreamAttributes(&self, _stream_id: u32) -> windows::core::Result<IMFAttributes> {
        self.descriptor.cast()
    }

    fn SetD3DManager(&self, _manager: Ref<IUnknown>) -> windows::core::Result<()> {
        Ok(())
    }
}

impl IMFGetService_Impl for MediaSource_Impl {
    fn GetService(
        &self,
        _service: *const GUID,
        _riid: *const GUID,
        _object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IKsControl_Impl for MediaSource_Impl {
    fn KsProperty(
        &self,
        _property: *const KSIDENTIFIER,
        _length: u32,
        _data: *mut c_void,
        _data_length: u32,
        _returned: *mut u32,
    ) -> windows::core::Result<()> {
        Err(SET_NOT_FOUND.into())
    }

    fn KsMethod(
        &self,
        _method: *const KSIDENTIFIER,
        _length: u32,
        _data: *mut c_void,
        _data_length: u32,
        _returned: *mut u32,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn KsEvent(
        &self,
        _event: *const KSIDENTIFIER,
        _length: u32,
        _data: *mut c_void,
        _data_length: u32,
        _returned: *mut u32,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
}
