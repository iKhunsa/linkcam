//! Where LinkCam keeps its files, and loading/saving the user configuration.

use linkcam_core::config::Config;
use std::path::PathBuf;

const APP_DIR: &str = "LinkCam";
const CONFIG_FILE: &str = "config.toml";

/// `%APPDATA%\LinkCam`, falling back to the current directory.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join(APP_DIR)
}

pub fn config_path() -> PathBuf {
    data_dir().join(CONFIG_FILE)
}

/// Loads the configuration; a missing or unreadable file yields the defaults.
pub fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|text| Config::from_toml(&text).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &Config) -> Result<(), String> {
    let text = config.to_toml()?;
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), text).map_err(|e| e.to_string())
}

/// Directory holding the gesture models: the configured one, or the first
/// candidate next to the executable (installed or `cargo run` layouts).
pub fn model_dir(configured: &str) -> Option<PathBuf> {
    const MODEL: &str = linkcam_vision::models::PALM_MODEL_FILE;
    let mut candidates = Vec::new();
    if !configured.is_empty() {
        candidates.push(PathBuf::from(configured));
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1).take(5) {
            candidates.push(ancestor.join("models"));
            candidates.push(ancestor.to_path_buf());
        }
    }
    candidates.into_iter().find(|dir| dir.join(MODEL).is_file())
}
