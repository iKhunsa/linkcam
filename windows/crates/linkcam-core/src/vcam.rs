//! Frame channel between the LinkCam app (producer) and the virtual-camera
//! media source loaded by Windows' Frame Server (consumer).
//!
//! The channel is one file: a small header followed by a top-down BGRA frame
//! of fixed size. The producer uses a sequence counter as a seqlock (odd while
//! it writes) so the consumer never shows a torn frame.

pub const WIDTH: usize = 1280;
pub const HEIGHT: usize = 720;
pub const FPS: u32 = 30;
pub const FRAME_BYTES: usize = WIDTH * HEIGHT * 4;

pub const MAGIC: u32 = 0x3143_4B4C; // "LKC1"
pub const HEADER_BYTES: usize = 32;

/// Friendly name of the camera as other applications list it.
pub const CAMERA_NAME: &str = "LinkCam Virtual Camera";

/// COM class of the media source (`{...}` form, as `MFCreateVirtualCamera` wants it).
pub const SOURCE_CLSID: &str = "{6F1D2E5A-3B7C-4D8E-9A10-5C4B3A291807}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub sequence: u32,
    pub timestamp_100ns: u64,
}

impl Header {
    pub fn encode(self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0u8; HEADER_BYTES];
        bytes[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[4..8].copy_from_slice(&(WIDTH as u32).to_le_bytes());
        bytes[8..12].copy_from_slice(&(HEIGHT as u32).to_le_bytes());
        bytes[12..16].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.timestamp_100ns.to_le_bytes());
        bytes
    }

    /// Parses a header; `None` when the magic or the frame size do not match.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_BYTES {
            return None;
        }
        let word =
            |at: usize| -> Option<u32> { Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)) };
        if word(0)? != MAGIC || word(4)? as usize != WIDTH || word(8)? as usize != HEIGHT {
            return None;
        }
        Some(Self {
            sequence: word(12)?,
            timestamp_100ns: u64::from_le_bytes(bytes.get(16..24)?.try_into().ok()?),
        })
    }

    /// A sequence is stable (fully written) when even.
    pub fn is_stable(self) -> bool {
        self.sequence.is_multiple_of(2)
    }
}

/// Fits an RGBA frame into the channel size, keeping the aspect ratio with
/// black bars, and returns BGRA bytes.
pub fn fit_rgba_to_channel(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; FRAME_BYTES];
    for pixel in out.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return out;
    }
    // Scale to fit inside WIDTH x HEIGHT.
    let scale = (WIDTH as f32 / width as f32).min(HEIGHT as f32 / height as f32);
    let (fit_w, fit_h) =
        (((width as f32 * scale) as usize).max(1), ((height as f32 * scale) as usize).max(1));
    let (left, top) = ((WIDTH - fit_w) / 2, (HEIGHT - fit_h) / 2);
    for y in 0..fit_h {
        let source_y = (y * height / fit_h).min(height - 1);
        for x in 0..fit_w {
            let source_x = (x * width / fit_w).min(width - 1);
            let src = (source_y * width + source_x) * 4;
            let dst = ((top + y) * WIDTH + left + x) * 4;
            out[dst] = rgba[src + 2];
            out[dst + 1] = rgba[src + 1];
            out[dst + 2] = rgba[src];
        }
    }
    out
}

/// BGRA -> NV12 (BT.601, limited range) for even dimensions.
pub fn bgra_to_nv12(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 3 / 2];
    let (luma, chroma) = out.split_at_mut(width * height);
    for y in 0..height {
        for x in 0..width {
            let p = (y * width + x) * 4;
            let (b, g, r) = (bgra[p] as i32, bgra[p + 1] as i32, bgra[p + 2] as i32);
            luma[y * width + x] = (16 + ((66 * r + 129 * g + 25 * b + 128) >> 8)).clamp(0, 255) as u8;
        }
    }
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let (mut b, mut g, mut r) = (0i32, 0i32, 0i32);
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let p = ((y + dy) * width + x + dx) * 4;
                b += bgra[p] as i32;
                g += bgra[p + 1] as i32;
                r += bgra[p + 2] as i32;
            }
            let (b, g, r) = (b / 4, g / 4, r / 4);
            let at = (y / 2) * width + x;
            chroma[at] = (128 + ((-38 * r - 74 * g + 112 * b + 128) >> 8)).clamp(0, 255) as u8;
            chroma[at + 1] = (128 + ((112 * r - 94 * g - 18 * b + 128) >> 8)).clamp(0, 255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_and_validation() {
        let header = Header { sequence: 42, timestamp_100ns: 123_456 };
        let bytes = header.encode();
        assert_eq!(Header::decode(&bytes), Some(header));
        assert!(header.is_stable());
        assert!(!Header { sequence: 43, ..header }.is_stable());

        let mut wrong = bytes;
        wrong[0] ^= 0xff;
        assert_eq!(Header::decode(&wrong), None);
        assert_eq!(Header::decode(&bytes[..10]), None);
    }

    #[test]
    fn fit_letterboxes_and_swaps_channels() {
        // 2x2 red image (RGBA) becomes a centred red square on black, in BGRA.
        let rgba = [255, 0, 0, 255].repeat(4);
        let bgra = fit_rgba_to_channel(&rgba, 2, 2);
        assert_eq!(bgra.len(), FRAME_BYTES);
        let centre = ((HEIGHT / 2) * WIDTH + WIDTH / 2) * 4;
        assert_eq!(&bgra[centre..centre + 4], &[0, 0, 255, 255]);
        assert_eq!(&bgra[0..4], &[0, 0, 0, 255], "corner is a black bar");
    }

    #[test]
    fn nv12_has_the_right_size_and_gray_levels() {
        let (w, h) = (4, 4);
        let white = [255u8, 255, 255, 255].repeat(w * h);
        let nv12 = bgra_to_nv12(&white, w, h);
        assert_eq!(nv12.len(), w * h * 3 / 2);
        assert!(nv12[..w * h].iter().all(|y| *y >= 234), "white luma is ~235");
        assert!(nv12[w * h..].iter().all(|c| (*c as i32 - 128).abs() <= 1), "no colour");
    }
}
