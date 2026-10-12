use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Keeps COM initialised on the current thread for as long as it lives.
pub struct ComGuard {
    initialised: bool,
}

impl ComGuard {
    pub fn init() -> Self {
        // S_FALSE (already initialised) is also success and still needs the matching uninit.
        let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self { initialised }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialised {
            unsafe { CoUninitialize() };
        }
    }
}
