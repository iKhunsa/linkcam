//! DirectShow enumeration of video capture devices.

use linkcam_core::{CameraError, Result};
use windows::core::{BSTR, PCWSTR};
use windows::Win32::Media::DirectShow::{IBaseFilter, ICreateDevEnum};
use windows::Win32::Media::MediaFoundation::{CLSID_SystemDeviceEnum, CLSID_VideoInputDeviceCategory};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{CoCreateInstance, IBindCtx, IEnumMoniker, IMoniker, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::VARIANT;

pub const INSTA360_VENDOR_ID: u16 = 0x2e1a;

/// Identity of one capture device as reported by DirectShow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInfo {
    pub name: String,
    pub device_path: String,
}

impl CameraInfo {
    /// Parses `vid_xxxx&pid_yyyy` out of the device path.
    pub fn vendor_product(&self) -> Option<(u16, u16)> {
        let path = self.device_path.to_ascii_lowercase();
        let hex_after = |tag: &str| {
            let start = path.find(tag)? + tag.len();
            u16::from_str_radix(path.get(start..start + 4)?, 16).ok()
        };
        Some((hex_after("vid_")?, hex_after("pid_")?))
    }

    pub fn is_insta360(&self) -> bool {
        self.vendor_product().is_some_and(|(vid, _)| vid == INSTA360_VENDOR_ID)
    }
}

/// Lists every video capture device together with its moniker.
fn enumerate() -> Result<Vec<(CameraInfo, IMoniker)>> {
    let read_err = |e: windows::core::Error| CameraError::ReadFailed(e.to_string());
    unsafe {
        let dev_enum: ICreateDevEnum =
            CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER).map_err(read_err)?;
        let mut monikers: Option<IEnumMoniker> = None;
        dev_enum
            .CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut monikers, 0)
            .map_err(read_err)?;
        let Some(monikers) = monikers else { return Ok(Vec::new()) };

        let mut found = Vec::new();
        loop {
            let mut slot = [None];
            let mut fetched = 0u32;
            if monikers.Next(&mut slot, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(moniker) = slot[0].take() else { break };
            if let Some(info) = read_info(&moniker) {
                found.push((info, moniker));
            }
        }
        Ok(found)
    }
}

unsafe fn read_info(moniker: &IMoniker) -> Option<CameraInfo> {
    let bag: IPropertyBag =
        unsafe { moniker.BindToStorage::<_, _, IPropertyBag>(None::<&IBindCtx>, None::<&IMoniker>) }.ok()?;
    let read = |key: &str| -> Option<String> {
        let wide: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        let mut value = VARIANT::default();
        unsafe { bag.Read(PCWSTR(wide.as_ptr()), &mut value, None) }.ok()?;
        BSTR::try_from(&value).ok().map(|b| b.to_string())
    };
    Some(CameraInfo {
        name: read("FriendlyName").unwrap_or_default(),
        device_path: read("DevicePath").unwrap_or_default(),
    })
}

pub fn list_cameras() -> Result<Vec<CameraInfo>> {
    Ok(enumerate()?.into_iter().map(|(info, _)| info).collect())
}

/// Binds the DirectShow capture filter of the first Insta360 camera.
pub(crate) fn open_insta360_filter() -> Result<(CameraInfo, IBaseFilter)> {
    let (info, moniker) =
        enumerate()?.into_iter().find(|(info, _)| info.is_insta360()).ok_or(CameraError::DeviceNotFound)?;
    let filter = unsafe { moniker.BindToObject::<_, _, IBaseFilter>(None::<&IBindCtx>, None::<&IMoniker>) }
        .map_err(|e| CameraError::ReadFailed(e.to_string()))?;
    Ok((info, filter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vendor_and_product_from_device_path() {
        let info = CameraInfo {
            name: "Insta360 Link 2C".into(),
            device_path:
                r"\\?\usb#vid_2e1a&pid_4c03&mi_00#6&2d757413&0&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}"
                    .into(),
        };
        assert_eq!(info.vendor_product(), Some((0x2e1a, 0x4c03)));
        assert!(info.is_insta360());
    }
}
