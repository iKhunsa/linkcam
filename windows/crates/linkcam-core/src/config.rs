//! User configuration persisted as TOML. Pure (de)serialisation; file
//! location and I/O live in the app crate.

use crate::format::VideoFormat;
use crate::gesture::{Action, Binding, CameraCommand, Gesture};
use crate::modes::VideoMode;
use crate::units::{AngleUnits, ZoomUnits};
use serde::{Deserialize, Serialize};

/// Which of the camera's built-in firmware gestures are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareGestures {
    pub auto_framing: bool,
    pub whiteboard: bool,
    pub zoom: bool,
}

impl Default for FirmwareGestures {
    fn default() -> Self {
        Self { auto_framing: true, whiteboard: true, zoom: true }
    }
}

/// Software effects applied to the preview, snapshots and recordings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectsConfig {
    pub blur_background: bool,
    /// 1 (light) ..= 5 (heavy).
    pub blur_strength: u8,
    pub overlay_enabled: bool,
    pub overlay_text: String,
}

impl Default for EffectsConfig {
    fn default() -> Self {
        Self {
            blur_background: false,
            blur_strength: 3,
            overlay_enabled: false,
            overlay_text: "LinkCam".into(),
        }
    }
}

impl EffectsConfig {
    pub fn any_active(&self) -> bool {
        self.blur_background || (self.overlay_enabled && !self.overlay_text.is_empty())
    }
}

/// Microphone options for recordings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub record_microphone: bool,
    /// Name fragment of the microphone to use; empty = automatic.
    pub microphone: String,
    pub noise_reduction: bool,
    /// Noise gate strength, 1.0 (mild) ..= 3.0 (strong).
    pub noise_strength: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            record_microphone: true,
            microphone: String::new(),
            noise_reduction: false,
            noise_strength: 2.0,
        }
    }
}

/// A saved camera view ("Scene preset" in the official controller).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePreset {
    pub slot: u8,
    pub pan: i32,
    pub tilt: i32,
    pub zoom: u16,
    pub mode: VideoMode,
}

impl ScenePreset {
    pub fn capture(slot: u8, pan: AngleUnits, tilt: AngleUnits, zoom: ZoomUnits, mode: VideoMode) -> Self {
        Self { slot, pan: pan.raw(), tilt: tilt.raw(), zoom: zoom.raw(), mode }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub firmware_gestures: FirmwareGestures,
    /// Master switch of the host-side gesture engine (uses the camera stream).
    pub custom_gestures_enabled: bool,
    pub bindings: Vec<Binding>,
    pub presets: Vec<ScenePreset>,
    /// Directory holding the MediaPipe ONNX models; empty = next to the exe.
    pub model_dir: String,
    /// Stream format chosen by the user; `None` = automatic (about 720p).
    pub preferred_format: Option<VideoFormat>,
    pub effects: EffectsConfig,
    pub audio: AudioConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            firmware_gestures: FirmwareGestures::default(),
            custom_gestures_enabled: false,
            bindings: default_bindings(),
            presets: Vec::new(),
            model_dir: String::new(),
            preferred_format: None,
            effects: EffectsConfig::default(),
            audio: AudioConfig::default(),
        }
    }
}

fn default_bindings() -> Vec<Binding> {
    let keys = |chord: &[&str]| Action::Keys { keys: chord.iter().map(|k| k.to_string()).collect() };
    vec![
        Binding::new(Gesture::SwipeRight, keys(&["win", "ctrl", "right"])),
        Binding::new(Gesture::SwipeLeft, keys(&["win", "ctrl", "left"])),
        Binding::new(Gesture::ThumbsUp, keys(&["media_play_pause"])),
        Binding::new(Gesture::Fist, Action::Camera(CameraCommand::Recenter)),
    ]
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Inserts or replaces the preset stored in `preset.slot`.
    pub fn upsert_preset(&mut self, preset: ScenePreset) {
        match self.presets.iter_mut().find(|p| p.slot == preset.slot) {
            Some(existing) => *existing = preset,
            None => self.presets.push(preset),
        }
    }

    pub fn preset(&self, slot: u8) -> Option<&ScenePreset> {
        self.presets.iter().find(|p| p.slot == slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips() {
        let config = Config::default();
        let parsed = Config::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let parsed = Config::from_toml("custom_gestures_enabled = true").unwrap();
        assert!(parsed.custom_gestures_enabled);
        assert_eq!(parsed.bindings, default_bindings());
    }

    #[test]
    fn effects_default_to_off_and_report_activity() {
        let mut effects = EffectsConfig::default();
        assert!(!effects.any_active());
        effects.overlay_enabled = true;
        assert!(effects.any_active());
        effects.overlay_text.clear();
        assert!(!effects.any_active());
    }

    #[test]
    fn readme_example_parses() {
        let text = r#"
custom_gestures_enabled = true

[[bindings]]
gesture = "peace"
hold_ms = 600
cooldown_ms = 1500
enabled = true
[bindings.action]
type = "keys"
keys = ["win", "ctrl", "right"]
"#;
        let config = Config::from_toml(text).unwrap();
        assert_eq!(config.bindings.len(), 1);
        assert_eq!(config.bindings[0].gesture, Gesture::Peace);
        assert!(matches!(&config.bindings[0].action, Action::Keys { keys } if keys.len() == 3));
    }

    #[test]
    fn upsert_replaces_same_slot() {
        let mut config = Config::default();
        let a =
            ScenePreset::capture(1, AngleUnits::ZERO, AngleUnits::ZERO, ZoomUnits::ONE_X, VideoMode::Normal);
        let b = ScenePreset { zoom: 200, ..a };
        config.upsert_preset(a);
        config.upsert_preset(b);
        assert_eq!(config.presets.len(), 1);
        assert_eq!(config.preset(1).unwrap().zoom, 200);
    }
}
