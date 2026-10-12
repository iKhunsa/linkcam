//! `IMFActivate` that Frame Server instantiates; it creates the media source.
//! Attribute methods are delegated to a plain `IMFAttributes` store.

use crate::source::MediaSource;
use std::ffi::c_void;
use windows::core::{implement, IUnknown, Interface, Ref, BOOL, GUID, PCWSTR, PWSTR};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

#[implement(IMFActivate)]
pub struct Activator {
    attributes: IMFAttributes,
}

impl Activator {
    pub fn create() -> windows::core::Result<IMFActivate> {
        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 0) }?;
        let attributes =
            attributes.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        Ok(Activator { attributes }.into())
    }
}

impl IMFActivate_Impl for Activator_Impl {
    fn ActivateObject(&self, riid: *const GUID, ppv: *mut *mut c_void) -> windows::core::Result<()> {
        let source: IMFMediaSourceEx = MediaSource::create()?;
        unsafe { source.query(riid, ppv).ok() }
    }

    fn ShutdownObject(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn DetachObject(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

type R<T> = windows::core::Result<T>;

impl IMFAttributes_Impl for Activator_Impl {
    fn GetItem(&self, key: *const GUID, value: *mut PROPVARIANT) -> R<()> {
        unsafe { self.attributes.GetItem(key, Some(value)) }
    }
    fn GetItemType(&self, key: *const GUID) -> R<MF_ATTRIBUTE_TYPE> {
        unsafe { self.attributes.GetItemType(key) }
    }
    fn CompareItem(&self, key: *const GUID, value: *const PROPVARIANT) -> R<BOOL> {
        unsafe { self.attributes.CompareItem(key, value) }
    }
    fn Compare(&self, theirs: Ref<IMFAttributes>, match_type: MF_ATTRIBUTES_MATCH_TYPE) -> R<BOOL> {
        unsafe { self.attributes.Compare(theirs.as_ref(), match_type) }
    }
    fn GetUINT32(&self, key: *const GUID) -> R<u32> {
        unsafe { self.attributes.GetUINT32(key) }
    }
    fn GetUINT64(&self, key: *const GUID) -> R<u64> {
        unsafe { self.attributes.GetUINT64(key) }
    }
    fn GetDouble(&self, key: *const GUID) -> R<f64> {
        unsafe { self.attributes.GetDouble(key) }
    }
    fn GetGUID(&self, key: *const GUID) -> R<GUID> {
        unsafe { self.attributes.GetGUID(key) }
    }
    fn GetStringLength(&self, key: *const GUID) -> R<u32> {
        unsafe { self.attributes.GetStringLength(key) }
    }
    fn GetString(&self, key: *const GUID, value: PWSTR, size: u32, length: *mut u32) -> R<()> {
        let buffer = unsafe { std::slice::from_raw_parts_mut(value.0, size as usize) };
        unsafe { self.attributes.GetString(key, buffer, Some(length)) }
    }
    fn GetAllocatedString(&self, key: *const GUID, value: *mut PWSTR, length: *mut u32) -> R<()> {
        unsafe { self.attributes.GetAllocatedString(key, value, length) }
    }
    fn GetBlobSize(&self, key: *const GUID) -> R<u32> {
        unsafe { self.attributes.GetBlobSize(key) }
    }
    fn GetBlob(&self, key: *const GUID, buffer: *mut u8, size: u32, blob_size: *mut u32) -> R<()> {
        let buffer = unsafe { std::slice::from_raw_parts_mut(buffer, size as usize) };
        unsafe { self.attributes.GetBlob(key, buffer, Some(blob_size)) }
    }
    fn GetAllocatedBlob(&self, key: *const GUID, buffer: *mut *mut u8, size: *mut u32) -> R<()> {
        unsafe { self.attributes.GetAllocatedBlob(key, buffer, size) }
    }
    fn GetUnknown(&self, key: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> R<()> {
        let unknown: IUnknown = unsafe { self.attributes.GetUnknown(key) }?;
        unsafe { unknown.query(riid, ppv).ok() }
    }
    fn SetItem(&self, key: *const GUID, value: *const PROPVARIANT) -> R<()> {
        unsafe { self.attributes.SetItem(key, value) }
    }
    fn DeleteItem(&self, key: *const GUID) -> R<()> {
        unsafe { self.attributes.DeleteItem(key) }
    }
    fn DeleteAllItems(&self) -> R<()> {
        unsafe { self.attributes.DeleteAllItems() }
    }
    fn SetUINT32(&self, key: *const GUID, value: u32) -> R<()> {
        unsafe { self.attributes.SetUINT32(key, value) }
    }
    fn SetUINT64(&self, key: *const GUID, value: u64) -> R<()> {
        unsafe { self.attributes.SetUINT64(key, value) }
    }
    fn SetDouble(&self, key: *const GUID, value: f64) -> R<()> {
        unsafe { self.attributes.SetDouble(key, value) }
    }
    fn SetGUID(&self, key: *const GUID, value: *const GUID) -> R<()> {
        unsafe { self.attributes.SetGUID(key, value) }
    }
    fn SetString(&self, key: *const GUID, value: &PCWSTR) -> R<()> {
        unsafe { self.attributes.SetString(key, *value) }
    }
    fn SetBlob(&self, key: *const GUID, buffer: *const u8, size: u32) -> R<()> {
        let buffer = unsafe { std::slice::from_raw_parts(buffer, size as usize) };
        unsafe { self.attributes.SetBlob(key, buffer) }
    }
    fn SetUnknown(&self, key: *const GUID, unknown: Ref<IUnknown>) -> R<()> {
        unsafe { self.attributes.SetUnknown(key, unknown.as_ref()) }
    }
    fn LockStore(&self) -> R<()> {
        unsafe { self.attributes.LockStore() }
    }
    fn UnlockStore(&self) -> R<()> {
        unsafe { self.attributes.UnlockStore() }
    }
    fn GetCount(&self) -> R<u32> {
        unsafe { self.attributes.GetCount() }
    }
    fn GetItemByIndex(&self, index: u32, key: *mut GUID, value: *mut PROPVARIANT) -> R<()> {
        unsafe { self.attributes.GetItemByIndex(index, key, Some(value)) }
    }
    fn CopyAllItems(&self, destination: Ref<IMFAttributes>) -> R<()> {
        unsafe { self.attributes.CopyAllItems(destination.as_ref()) }
    }
}
