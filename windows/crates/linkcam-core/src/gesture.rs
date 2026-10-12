//! Gestures, actions and the bindings that connect them.

use crate::modes::VideoMode;
use serde::{Deserialize, Serialize};

/// Hand gestures the host-side engine can recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    OpenPalm,
    Fist,
    ThumbsUp,
    Peace,
    Point,
    Pinch,
    SwipeLeft,
    SwipeRight,
    SwipeUp,
    SwipeDown,
}

impl Gesture {
    pub const ALL: [Self; 10] = [
        Self::OpenPalm,
        Self::Fist,
        Self::ThumbsUp,
        Self::Peace,
        Self::Point,
        Self::Pinch,
        Self::SwipeLeft,
        Self::SwipeRight,
        Self::SwipeUp,
        Self::SwipeDown,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenPalm => "Open palm",
            Self::Fist => "Fist",
            Self::ThumbsUp => "Thumbs up",
            Self::Peace => "Peace / V",
            Self::Point => "Point",
            Self::Pinch => "Pinch",
            Self::SwipeLeft => "Swipe left",
            Self::SwipeRight => "Swipe right",
            Self::SwipeUp => "Swipe up",
            Self::SwipeDown => "Swipe down",
        }
    }

    /// Swipes are motions, so they fire immediately instead of after a hold.
    pub const fn is_motion(self) -> bool {
        matches!(self, Self::SwipeLeft | Self::SwipeRight | Self::SwipeUp | Self::SwipeDown)
    }
}

/// Camera-side effect a gesture can trigger without touching the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CameraCommand {
    SetMode { mode: VideoMode },
    ToggleMode { mode: VideoMode },
    ZoomBy { percent: i16 },
    Recenter,
}

/// What happens when a gesture fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Press a chord such as `["ctrl", "alt", "t"]`.
    Keys {
        keys: Vec<String>,
    },
    /// Start a program or open a document/URL.
    Launch {
        target: String,
    },
    Camera(CameraCommand),
}

/// Links one gesture to one action, with timing guards against repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub gesture: Gesture,
    pub action: Action,
    /// How long the pose must be held before firing (ignored for swipes).
    #[serde(default = "default_hold_ms")]
    pub hold_ms: u32,
    /// Minimum time before the same binding can fire again.
    #[serde(default = "default_cooldown_ms")]
    pub cooldown_ms: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

const fn default_hold_ms() -> u32 {
    600
}
const fn default_cooldown_ms() -> u32 {
    1500
}
const fn default_enabled() -> bool {
    true
}

impl Binding {
    pub fn new(gesture: Gesture, action: Action) -> Self {
        Self {
            gesture,
            action,
            hold_ms: default_hold_ms(),
            cooldown_ms: default_cooldown_ms(),
            enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_roundtrips_through_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Wrapper {
            binding: Vec<Binding>,
        }
        let original = Wrapper {
            binding: vec![
                Binding::new(
                    Gesture::Peace,
                    Action::Keys { keys: vec!["win".into(), "ctrl".into(), "right".into()] },
                ),
                Binding::new(
                    Gesture::Fist,
                    Action::Camera(CameraCommand::SetMode { mode: VideoMode::Normal }),
                ),
            ],
        };
        let text = toml::to_string(&original).unwrap();
        let parsed: Wrapper = toml::from_str(&text).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn only_swipes_are_motions() {
        let motions: Vec<_> = Gesture::ALL.into_iter().filter(|g| g.is_motion()).collect();
        assert_eq!(motions.len(), 4);
    }
}
