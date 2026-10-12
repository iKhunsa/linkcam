//! Start-with-Windows through the per-user `Run` registry key.

use linkcam_core::{CameraError, Result};
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "LinkCam";

fn reg(args: &[&str]) -> Result<std::process::Output> {
    Command::new("reg").args(args).output().map_err(|e| CameraError::WriteFailed(format!("reg.exe: {e}")))
}

pub fn is_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", VALUE_NAME]).is_ok_and(|o| o.status.success())
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    let output = if enabled {
        let exe = std::env::current_exe().map_err(|e| CameraError::WriteFailed(e.to_string()))?;
        let command = format!("\"{}\"", exe.display());
        reg(&["add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &command, "/f"])?
    } else {
        reg(&["delete", RUN_KEY, "/v", VALUE_NAME, "/f"])?
    };
    if output.status.success() || !enabled {
        Ok(())
    } else {
        Err(CameraError::WriteFailed(String::from_utf8_lossy(&output.stderr).trim().to_string()))
    }
}
