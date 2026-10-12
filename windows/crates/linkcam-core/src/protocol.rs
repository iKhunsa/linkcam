//! Vendor extension-unit (XU) protocol of the Link 2C: selectors and payload
//! encoding. Every constant was measured on hardware; see `docs/xu-map-link2c.md`.

use crate::modes::VideoMode;

/// Index into the known XU GUID list: the primary unit (`FAF1672D-...`).
pub const PRIMARY_XU_GUID_INDEX: usize = 0;
/// Video mode selector. The payload is 61 bytes on the 2C and must be
/// written back whole (read-modify-write).
pub const SELECTOR_VIDEO_MODE: u8 = 0x02;
/// Smallest payload we will accept for the mode selector (byte 0 and 1 are touched).
pub const MIN_MODE_PAYLOAD_LEN: usize = 2;
/// Byte 0 of a rejected / not-streaming readback.
pub const MODE_REJECTED: u8 = 0xff;

/// Returns `payload` with the mode id and flag bytes replaced; all other
/// bytes (the camera's own geometry block) are preserved.
pub fn payload_with_mode(current: &[u8], mode: VideoMode) -> Option<Vec<u8>> {
    if current.len() < MIN_MODE_PAYLOAD_LEN {
        return None;
    }
    let mut payload = current.to_vec();
    payload[0] = mode.id();
    payload[1] = mode.flag();
    Some(payload)
}

/// Decodes the mode echoed in byte 0. `None` means rejected or unknown.
pub fn mode_from_payload(payload: &[u8]) -> Option<VideoMode> {
    VideoMode::from_id(*payload.first()?)
}

pub fn write_i32_le(buffer: &mut [u8], offset: usize, value: i32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn read_i32_le(buffer: &[u8], offset: usize) -> Option<i32> {
    let bytes: [u8; 4] = buffer.get(offset..offset + 4)?.try_into().ok()?;
    Some(i32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_payload() -> Vec<u8> {
        // Captured from a Link 2C in normal mode.
        let hex = "0000000000000000000000000000000000000000000000000000000000000000000000000000ff7fff7f000000000000000064003efe22280000000001";
        (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap()).collect()
    }

    #[test]
    fn mode_switch_changes_only_the_first_two_bytes() {
        let original = real_payload();
        let patched = payload_with_mode(&original, VideoMode::DeskView).unwrap();
        assert_eq!(patched.len(), original.len());
        assert_eq!(&patched[..2], &[6, 0x10]);
        assert_eq!(&patched[2..], &original[2..]);
    }

    #[test]
    fn echo_decoding() {
        assert_eq!(mode_from_payload(&[6, 0x10]), Some(VideoMode::DeskView));
        assert_eq!(mode_from_payload(&[0xff, 0]), None);
        assert_eq!(mode_from_payload(&[]), None);
    }

    #[test]
    fn too_short_payload_is_refused() {
        assert!(payload_with_mode(&[0], VideoMode::Normal).is_none());
    }
}
