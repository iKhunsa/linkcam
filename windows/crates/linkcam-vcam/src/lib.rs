//! LinkCam virtual camera: a Media Foundation media source (loaded by Windows'
//! Frame Server through `MFCreateVirtualCamera`) that shows the frames the app
//! publishes on the file channel.
//!
//! COM entry points only; the objects live in `activator`, `source` and `stream`.

mod activator;
mod source;
mod stream;

use std::ffi::c_void;
use windows::core::{implement, IUnknown, Interface, Ref, BOOL, GUID, HRESULT};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, S_FALSE};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};

/// Must match `linkcam_core::vcam::SOURCE_CLSID`.
pub const SOURCE_CLSID: GUID = GUID::from_u128(0x6f1d2e5a_3b7c_4d8e_9a10_5c4b3a291807);

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activator: windows::Win32::Media::MediaFoundation::IMFActivate = activator::Activator::create()?;
        unsafe { activator.query(riid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

/// # Safety
/// Called by COM with valid pointers.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || object.is_null() {
        return windows::Win32::Foundation::E_POINTER;
    }
    if unsafe { *clsid } != SOURCE_CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    unsafe { factory.query(iid, object) }
}

/// The DLL stays loaded while Frame Server uses it.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;
    use linkcam_core::vcam::{FRAME_BYTES, HEIGHT, WIDTH};
    use linkcam_win::vcam_channel::FrameWriter;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    /// Left half red, right half blue, BGRA.
    fn test_card() -> Vec<u8> {
        let mut bgra = vec![0u8; FRAME_BYTES];
        for index in 0..WIDTH * HEIGHT {
            let colour = if index % WIDTH < WIDTH / 2 { [0, 0, 255, 255] } else { [255, 0, 0, 255] };
            bgra[index * 4..index * 4 + 4].copy_from_slice(&colour);
        }
        bgra
    }

    /// Activates the media source exactly as Frame Server would (class factory ->
    /// IMFActivate -> IMFMediaSource) and reads frames with a source reader.
    #[test]
    fn media_source_delivers_the_published_frames() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap();

            let mut writer = FrameWriter::create().unwrap();
            let card = test_card();
            writer.publish(&card).unwrap();

            let mut factory_ptr = std::ptr::null_mut();
            DllGetClassObject(&SOURCE_CLSID, &IClassFactory::IID, &mut factory_ptr).ok().unwrap();
            let factory = IClassFactory::from_raw(factory_ptr);
            let activate: IMFActivate = factory.CreateInstance(None).unwrap();
            let source: IMFMediaSource = activate.ActivateObject().unwrap();

            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1).unwrap();
            let attributes = attributes.unwrap();
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).unwrap();
            let reader = MFCreateSourceReaderFromMediaSource(&source, &attributes).unwrap();

            let output = MFCreateMediaType().unwrap();
            output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
            output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).unwrap();
            reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &output).unwrap();

            let mut pixel_left = [0u8; 3];
            let mut pixel_right = [0u8; 3];
            let mut frames = 0;
            for _ in 0..30 {
                writer.publish(&card).unwrap();
                let mut sample = None;
                let mut flags = 0u32;
                reader
                    .ReadSample(
                        MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                        0,
                        None,
                        Some(&mut flags),
                        None,
                        Some(&mut sample),
                    )
                    .unwrap();
                let Some(sample) = sample else { continue };
                let buffer = sample.ConvertToContiguousBuffer().unwrap();
                let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
                buffer.Lock(&mut data, None, Some(&mut length)).unwrap();
                let bytes = std::slice::from_raw_parts(data, length as usize);
                let at = |x: usize| ((HEIGHT / 2) * WIDTH + x) * 4;
                pixel_left.copy_from_slice(&bytes[at(WIDTH / 4)..at(WIDTH / 4) + 3]);
                pixel_right.copy_from_slice(&bytes[at(3 * WIDTH / 4)..at(3 * WIDTH / 4) + 3]);
                buffer.Unlock().unwrap();
                frames += 1;
                if frames >= 5 {
                    break;
                }
            }
            source.Shutdown().unwrap();

            assert!(frames >= 5, "got only {frames} frames");
            // RGB32 is B,G,R,X: red is (0,0,255), blue is (255,0,0).
            let close = |a: u8, b: u8| (a as i32 - b as i32).abs() < 40;
            assert!(close(pixel_left[2], 255) && close(pixel_left[0], 0), "left pixel {pixel_left:?}");
            assert!(close(pixel_right[0], 255) && close(pixel_right[2], 0), "right pixel {pixel_right:?}");
        }
    }
}
