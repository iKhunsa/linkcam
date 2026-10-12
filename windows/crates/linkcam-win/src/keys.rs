//! Keyboard injection: turns chords such as `["ctrl", "alt", "t"]` into
//! `SendInput` events.

use linkcam_core::{CameraError, Result};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// Resolves a key name (case-insensitive) to a virtual-key code.
pub fn virtual_key(name: &str) -> Option<u16> {
    let lower = name.trim().to_ascii_lowercase();
    let named = match lower.as_str() {
        "ctrl" | "control" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" => VK_MENU,
        "win" | "super" | "meta" => VK_LWIN,
        "enter" | "return" => VK_RETURN,
        "esc" | "escape" => VK_ESCAPE,
        "tab" => VK_TAB,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "insert" => VK_INSERT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "printscreen" | "prtsc" => VK_SNAPSHOT,
        "media_play_pause" => VK_MEDIA_PLAY_PAUSE,
        "media_next" => VK_MEDIA_NEXT_TRACK,
        "media_prev" => VK_MEDIA_PREV_TRACK,
        "media_stop" => VK_MEDIA_STOP,
        "volume_up" => VK_VOLUME_UP,
        "volume_down" => VK_VOLUME_DOWN,
        "volume_mute" => VK_VOLUME_MUTE,
        _ => return single_char_or_function_key(&lower),
    };
    Some(named.0)
}

fn single_char_or_function_key(lower: &str) -> Option<u16> {
    if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        return (1..=24).contains(&number).then(|| VK_F1.0 + number - 1);
    }
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(c @ 'a'..='z'), None) => Some(c.to_ascii_uppercase() as u16),
        (Some(c @ '0'..='9'), None) => Some(c as u16),
        _ => None,
    }
}

/// Keys that must be sent with the extended-key flag to be interpreted correctly.
fn is_extended(vk: u16) -> bool {
    [VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_INSERT, VK_DELETE, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_LWIN]
        .iter()
        .any(|k| k.0 == vk)
}

fn key_event(vk: u16, key_up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    if is_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

/// Presses the chord: keys go down in order and come up in reverse.
pub fn press_chord(names: &[String]) -> Result<()> {
    let codes: Vec<u16> = names
        .iter()
        .map(|n| virtual_key(n).ok_or_else(|| CameraError::InvalidArgument(format!("unknown key `{n}`"))))
        .collect::<Result<_>>()?;
    if codes.is_empty() {
        return Err(CameraError::InvalidArgument("empty key chord".into()));
    }
    let events: Vec<INPUT> = codes
        .iter()
        .map(|&vk| key_event(vk, false))
        .chain(codes.iter().rev().map(|&vk| key_event(vk, true)))
        .collect();
    let sent = unsafe { SendInput(&events, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == events.len() {
        Ok(())
    } else {
        Err(CameraError::WriteFailed("SendInput was blocked (is the target elevated?)".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_modifiers_letters_and_function_keys() {
        assert_eq!(virtual_key("Ctrl"), Some(VK_CONTROL.0));
        assert_eq!(virtual_key("t"), Some('T' as u16));
        assert_eq!(virtual_key("5"), Some('5' as u16));
        assert_eq!(virtual_key("F12"), Some(VK_F1.0 + 11));
        assert_eq!(virtual_key("f25"), None);
        assert_eq!(virtual_key("media_play_pause"), Some(VK_MEDIA_PLAY_PAUSE.0));
        assert_eq!(virtual_key("nonsense"), None);
    }

    #[test]
    fn unknown_key_is_rejected_before_sending() {
        assert!(press_chord(&["ctrl".into(), "banana".into()]).is_err());
        assert!(press_chord(&[]).is_err());
    }
}
