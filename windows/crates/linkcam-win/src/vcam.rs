//! Registers and runs the LinkCam virtual camera (Windows 11, Media Foundation).
//!
//! The media source lives in `linkcam_vcam.dll`. Windows' Frame Server resolves
//! its COM class system-wide, so the one-time registration is written to HKLM
//! and needs administrator approval (a UAC prompt, launched on user request).
//! Creating and running the camera afterwards needs no elevation.

use crate::vcam_channel::{channel_path, FrameWriter};
use linkcam_core::vcam::{fit_rgba_to_channel, CAMERA_NAME, SOURCE_CLSID};
use linkcam_core::{CameraError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::HSTRING;
use windows::Win32::Media::MediaFoundation::*;

pub const DLL_NAME: &str = "linkcam_vcam.dll";

fn fail(what: &str) -> impl Fn(windows::core::Error) -> CameraError + '_ {
    move |e| CameraError::WriteFailed(format!("{what}: {e}"))
}

fn class_key() -> String {
    format!(r"HKLM\SOFTWARE\Classes\CLSID\{SOURCE_CLSID}")
}

fn server_key() -> String {
    format!(r"{}\InprocServer32", class_key())
}

/// Copies the DLL next to the channel file: Frame Server runs under a service
/// account that cannot read the user's profile folders.
pub fn stage_dll(exe_dir: &Path) -> Result<PathBuf> {
    let source = exe_dir.join(DLL_NAME);
    if !source.is_file() {
        return Err(CameraError::ControlUnavailable("linkcam_vcam.dll next to linkcam.exe"));
    }
    let staged = channel_path().with_file_name(DLL_NAME);
    if let Some(dir) = staged.parent() {
        std::fs::create_dir_all(dir).map_err(|e| CameraError::WriteFailed(e.to_string()))?;
    }
    std::fs::copy(&source, &staged).map_err(|e| CameraError::WriteFailed(format!("staging the DLL: {e}")))?;
    Ok(staged)
}

/// PowerShell command line that elevates once (UAC) and runs the script file.
pub fn elevated_command(script_path: &Path) -> String {
    let file = script_path.display().to_string().replace('\'', "''");
    format!(
        "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','{file}'"
    )
}

fn registration_script(dll: &Path) -> String {
    let key = server_key();
    let path = dll.display();
    format!(
        "reg add \"{key}\" /ve /t REG_SZ /d \"{path}\" /f; reg add \"{key}\" /v ThreadingModel /t REG_SZ /d Both /f"
    )
}

/// Writes `script` to a file next to the channel and runs it elevated (one UAC
/// prompt). A file avoids every quoting problem of `-Command`; the transcript
/// next to it shows what the elevated step did.
fn run_elevated(script: &str) -> Result<()> {
    let path = channel_path().with_file_name("elevated-step.ps1");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| CameraError::WriteFailed(e.to_string()))?;
    }
    let log = path.with_extension("log");
    let body = format!(
        "Start-Transcript -Path '{}' -Force | Out-Null\r\n{script}\r\nStop-Transcript | Out-Null\r\n",
        log.display()
    );
    std::fs::write(&path, body).map_err(|e| CameraError::WriteFailed(e.to_string()))?;
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", &elevated_command(&path)])
        .status()
        .map_err(|e| CameraError::WriteFailed(format!("powershell: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CameraError::WriteFailed("administrator approval was not granted".into()))
    }
}

/// Registers `dll` for all users. Shows a UAC prompt; returns once it is done.
pub fn register(dll: &Path) -> Result<()> {
    run_elevated(&registration_script(dll))?;
    if is_registered() {
        Ok(())
    } else {
        Err(CameraError::WriteFailed("registration did not take effect".into()))
    }
}

pub fn unregister() -> Result<()> {
    run_elevated(&format!("reg delete \"{}\" /f", class_key()))
}

pub fn is_registered() -> bool {
    Command::new("reg")
        .args(["query", &server_key(), "/ve"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A running virtual camera; it disappears when this value is dropped.
pub struct VirtualCamera {
    camera: IMFVirtualCamera,
    writer: FrameWriter,
}

impl VirtualCamera {
    /// Creates and starts the camera (the class must be registered).
    pub fn start() -> Result<Self> {
        if !is_registered() {
            return Err(CameraError::ControlUnavailable(
                "virtual camera registration (run `linkcam vcam install` once)",
            ));
        }
        let writer = FrameWriter::create()?;
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(fail("MFStartup"))?;
            let camera = MFCreateVirtualCamera(
                MFVirtualCameraType_SoftwareCameraSource,
                MFVirtualCameraLifetime_Session,
                MFVirtualCameraAccess_CurrentUser,
                &HSTRING::from(CAMERA_NAME),
                &HSTRING::from(SOURCE_CLSID),
                None,
            )
            .map_err(fail("MFCreateVirtualCamera"))?;
            camera.Start(None).map_err(fail("virtual camera start"))?;
            Ok(Self { camera, writer })
        }
    }

    /// Publishes an RGBA frame (any size; letterboxed into the channel).
    pub fn publish(&mut self, rgba: &[u8], width: usize, height: usize) -> Result<()> {
        self.writer.publish(&fit_rgba_to_channel(rgba, width, height))
    }
}

impl Drop for VirtualCamera {
    fn drop(&mut self) {
        unsafe {
            let _ = self.camera.Stop();
            let _ = self.camera.Remove();
            let _ = MFShutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_script_writes_the_server_and_threading_model() {
        let script = registration_script(Path::new(r"C:\Users\Public\LinkCam\linkcam_vcam.dll"));
        assert!(script
            .contains(r"HKLM\SOFTWARE\Classes\CLSID\{6F1D2E5A-3B7C-4D8E-9A10-5C4B3A291807}\InprocServer32"));
        assert!(script.contains("ThreadingModel"));
        assert!(script.contains(r"C:\Users\Public\LinkCam\linkcam_vcam.dll"));
    }

    #[test]
    fn elevated_command_runs_the_script_file_as_administrator() {
        let command = elevated_command(Path::new(r"C:\Users\Public\LinkCam\step's.ps1"));
        assert!(command.contains("-Verb RunAs"));
        assert!(command.contains("'-File'"));
        assert!(command.contains("step''s.ps1"), "single quotes are doubled: {command}");
    }
}
