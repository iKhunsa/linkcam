//! Software effects applied to RGBA frames: background blur from a person
//! mask, and bitmap-font text overlays.

use crate::segmentation::Mask;
use font8x8::{UnicodeFonts, BASIC_FONTS};

/// The background is blurred on a copy reduced by this factor, then upsampled.
const BLUR_DOWNSCALE: usize = 4;
/// Box-blur radius (in reduced pixels) per strength step.
const BLUR_RADIUS_PER_STEP: usize = 3;
/// Passes of the box blur; three approximate a Gaussian well.
const BLUR_PASSES: usize = 3;
const GLYPH_SIZE: usize = 8;

/// Blurs everything the mask says is not a person. `strength` is 1..=5.
pub fn blur_background(rgba: &mut [u8], width: usize, height: usize, mask: &Mask, strength: u8) {
    assert_eq!(rgba.len(), width * height * 4, "frame size mismatch");
    let small_w = (width / BLUR_DOWNSCALE).max(1);
    let small_h = (height / BLUR_DOWNSCALE).max(1);
    let radius = usize::from(strength.clamp(1, 5)) * BLUR_RADIUS_PER_STEP;
    let blurred = blurred_copy(rgba, width, height, small_w, small_h, radius);

    for y in 0..height {
        let v = y as f32 / (height - 1).max(1) as f32;
        for x in 0..width {
            let u = x as f32 / (width - 1).max(1) as f32;
            let person = mask.sample(u, v).clamp(0.0, 1.0);
            if person >= 0.999 {
                continue;
            }
            let background = sample_rgb(&blurred, small_w, small_h, u, v);
            let at = (y * width + x) * 4;
            for (channel, bg) in background.iter().enumerate() {
                let original = rgba[at + channel] as f32;
                rgba[at + channel] = (original * person + bg * (1.0 - person)).round() as u8;
            }
        }
    }
}

/// Area-average downscale followed by a separable box blur.
fn blurred_copy(
    rgba: &[u8],
    width: usize,
    height: usize,
    small_w: usize,
    small_h: usize,
    radius: usize,
) -> Vec<f32> {
    let mut small = vec![0.0f32; small_w * small_h * 3];
    for y in 0..small_h {
        for x in 0..small_w {
            let (x0, x1) = (x * width / small_w, ((x + 1) * width / small_w).max(x * width / small_w + 1));
            let (y0, y1) = (y * height / small_h, ((y + 1) * height / small_h).max(y * height / small_h + 1));
            let mut sum = [0.0f32; 3];
            for sy in y0..y1.min(height) {
                for sx in x0..x1.min(width) {
                    let at = (sy * width + sx) * 4;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += rgba[at + c] as f32;
                    }
                }
            }
            let count = ((y1.min(height) - y0) * (x1.min(width) - x0)).max(1) as f32;
            for (c, total) in sum.iter().enumerate() {
                small[(y * small_w + x) * 3 + c] = total / count;
            }
        }
    }
    for _ in 0..BLUR_PASSES {
        box_blur_axis(&mut small, small_w, small_h, radius, true);
        box_blur_axis(&mut small, small_w, small_h, radius, false);
    }
    small
}

/// Running-sum box blur along one axis with edge clamping.
fn box_blur_axis(data: &mut [f32], width: usize, height: usize, radius: usize, horizontal: bool) {
    let (lines, length) = if horizontal { (height, width) } else { (width, height) };
    let index = |line: usize, i: usize| {
        if horizontal {
            (line * width + i) * 3
        } else {
            (i * width + line) * 3
        }
    };
    let window = (2 * radius + 1) as f32;
    let mut original = vec![0.0f32; length * 3];
    for line in 0..lines {
        for i in 0..length {
            original[i * 3..i * 3 + 3].copy_from_slice(&data[index(line, i)..index(line, i) + 3]);
        }
        let clamp = |i: isize| i.clamp(0, length as isize - 1) as usize;
        let mut sum = [0.0f32; 3];
        for offset in -(radius as isize)..=radius as isize {
            let at = clamp(offset) * 3;
            for c in 0..3 {
                sum[c] += original[at + c];
            }
        }
        for i in 0..length {
            let at = index(line, i);
            for c in 0..3 {
                data[at + c] = sum[c] / window;
            }
            let leaving = clamp(i as isize - radius as isize) * 3;
            let entering = clamp(i as isize + radius as isize + 1) * 3;
            for c in 0..3 {
                sum[c] += original[entering + c] - original[leaving + c];
            }
        }
    }
}

fn sample_rgb(data: &[f32], width: usize, height: usize, u: f32, v: f32) -> [f32; 3] {
    let x = (u * (width - 1) as f32).clamp(0.0, (width - 1) as f32);
    let y = (v * (height - 1) as f32).clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |px: usize, py: usize, c: usize| data[(py * width + px) * 3 + c];
    let mut out = [0.0; 3];
    for (c, value) in out.iter_mut().enumerate() {
        let top = at(x0, y0, c) * (1.0 - fx) + at(x1, y0, c) * fx;
        let bottom = at(x0, y1, c) * (1.0 - fx) + at(x1, y1, c) * fx;
        *value = top * (1.0 - fy) + bottom * fy;
    }
    out
}

/// Draws `text` with the built-in 8x8 font, `scale` times enlarged, white with a
/// dark outline so it reads on any background. Anchored at the bottom-left.
pub fn draw_text(rgba: &mut [u8], width: usize, height: usize, text: &str, scale: usize, margin: usize) {
    let scale = scale.max(1);
    let glyph_px = GLYPH_SIZE * scale;
    let top = height.saturating_sub(margin + glyph_px);
    let mut pen_x = margin;
    for ch in text.chars() {
        let Some(glyph) = BASIC_FONTS.get(ch) else {
            pen_x += glyph_px;
            continue;
        };
        if pen_x + glyph_px > width {
            break;
        }
        // Outline first, so the fill is drawn on top of it.
        for (pass, color) in [(true, [0u8, 0, 0]), (false, [255u8, 255, 255])] {
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..GLYPH_SIZE {
                    if bits & (1 << col) == 0 {
                        continue;
                    }
                    let (gx, gy) = (pen_x + col * scale, top + row * scale);
                    let spread = if pass { scale } else { 0 };
                    paint_block(
                        rgba,
                        width,
                        height,
                        gx.saturating_sub(spread),
                        gy.saturating_sub(spread),
                        scale + 2 * spread,
                        color,
                    );
                }
            }
        }
        pen_x += glyph_px;
    }
}

fn paint_block(
    rgba: &mut [u8],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    size: usize,
    color: [u8; 3],
) {
    for py in y..(y + size).min(height) {
        for px in x..(x + size).min(width) {
            let at = (py * width + px) * 4;
            rgba[at..at + 3].copy_from_slice(&color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize) -> Vec<u8> {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let v = if (x / 2 + y / 2) % 2 == 0 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        rgba
    }

    #[test]
    fn person_pixels_are_untouched_and_background_is_smoothed() {
        let (w, h) = (64, 64);
        let original = gradient(w, h);
        let mut frame = original.clone();
        // Person on the left half only.
        let mut mask = Mask::filled(2, 1, 1.0);
        mask.person[1] = 0.0;
        blur_background(&mut frame, w, h, &mask, 2);

        let left = (10 * w) * 4;
        assert_eq!(frame[left..left + 4], original[left..left + 4], "person side unchanged");
        // Checkerboard turned into mid grey on the background side.
        let right = (10 * w + 60) * 4;
        assert!((frame[right] as i32 - 128).abs() < 60, "background blurred, got {}", frame[right]);
    }

    #[test]
    fn text_marks_pixels_in_the_bottom_left() {
        let (w, h) = (80, 40);
        let mut frame = vec![0u8; w * h * 4];
        draw_text(&mut frame, w, h, "A", 2, 4);
        let any_white = (0..w * h).any(|i| frame[i * 4..i * 4 + 3] == [255, 255, 255]);
        assert!(any_white);
        // Nothing is drawn in the top half.
        assert!(frame[..w * (h / 2) * 4].iter().all(|b| *b == 0));
    }

    #[test]
    fn text_longer_than_the_frame_is_clipped_not_panicking() {
        let mut frame = vec![0u8; 32 * 16 * 4];
        draw_text(&mut frame, 32, 16, "this text is far too long for the frame", 2, 2);
    }
}
