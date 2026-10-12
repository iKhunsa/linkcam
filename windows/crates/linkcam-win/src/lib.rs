//! Windows backend: DirectShow device discovery, vendor extension-unit
//! access through `IKsControl`, and standard camera controls.
//!
//! COM objects are apartment-bound: create and use a `WinCamera` on a
//! single thread (the app runs it on a dedicated camera thread).

mod camera;
mod com;
mod controls;
mod discovery;
mod xu;

pub use camera::{ProbeHandle, WinCamera};
pub use com::ComGuard;
pub use controls::PtzAxis;
pub use discovery::{list_cameras, CameraInfo, INSTA360_VENDOR_ID};
pub use xu::{XuAccess, XuClient, XuGuid, KNOWN_XU_GUIDS};

pub mod keys;
pub mod preview;
pub mod record;
