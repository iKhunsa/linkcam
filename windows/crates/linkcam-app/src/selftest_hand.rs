//! Pushes a real photo of an open hand through the whole application stack as
//! if it were a live camera: virtual camera -> Media Foundation capture ->
//! camera service -> hand models -> classifier -> dispatcher -> key press.
//!
//! This stands in for a person in front of the lens, so the gesture feature
//! can be checked even when the physical camera is busy.

use crate::service::{Event, Service};
use linkcam_core::config::Config;
use linkcam_core::gesture::{Action, Binding, Gesture};
use linkcam_win::vcam::{self, VirtualCamera};
use linkcam_win::ComGuard;
use std::path::Path;
use std::time::{Duration, Instant};

/// Capture-device name fragment of the LinkCam virtual camera.
const VIRTUAL_SOURCE: &str = "LinkCam Virtual";
const WAIT: Duration = Duration::from_secs(45);
const FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// F24 exists on no keyboard, so pressing it has no side effect.
const HARMLESS_KEY: &str = "f24";

fn load_rgba(path: &Path) -> Result<(usize, usize, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("no JPEG info")?;
    let rgba = pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    Ok((info.width as usize, info.height as usize, rgba))
}

/// Scales `rgba` to cover the virtual camera's frame (cropping the overflow), so
/// the hand is as large as it would be close to a real lens.
fn cover_frame(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let (out_w, out_h) = (linkcam_core::vcam::WIDTH, linkcam_core::vcam::HEIGHT);
    let scale = (out_w as f32 / width as f32).max(out_h as f32 / height as f32);
    let (crop_x, crop_y) =
        ((width as f32 * scale - out_w as f32) / 2.0, (height as f32 * scale - out_h as f32) / 2.0);
    let mut out = vec![255u8; out_w * out_h * 4];
    for y in 0..out_h {
        let source_y = (((y as f32 + crop_y) / scale) as usize).min(height - 1);
        for x in 0..out_w {
            let source_x = (((x as f32 + crop_x) / scale) as usize).min(width - 1);
            let from = (source_y * width + source_x) * 4;
            let to = (y * out_w + x) * 4;
            out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    out
}

/// Returns `(passed, detail)`.
pub fn run(photo: &Path) -> (bool, String) {
    if !vcam::is_registered() {
        return (false, "virtual camera not registered (run `linkcam vcam install`)".into());
    }
    let (width, height, rgba) = match load_rgba(photo) {
        Ok(image) => image,
        Err(error) => return (false, error),
    };
    let frame = cover_frame(&rgba, width, height);
    let (width, height) = (linkcam_core::vcam::WIDTH, linkcam_core::vcam::HEIGHT);
    let _com = ComGuard::init();
    let mut virtual_camera = match VirtualCamera::start() {
        Ok(camera) => camera,
        Err(error) => return (false, error.to_string()),
    };

    let mut binding = Binding::new(Gesture::OpenPalm, Action::Keys { keys: vec![HARMLESS_KEY.into()] });
    binding.hold_ms = 300;
    binding.cooldown_ms = 1000;
    let config = Config {
        custom_gestures_enabled: true,
        bindings: vec![binding],
        video_source: VIRTUAL_SOURCE.into(),
        ..Config::default()
    };
    let service = Service::spawn(config, || {});

    let deadline = Instant::now() + WAIT;
    let mut last_error = None;
    let (mut rounds, mut frames, mut last_pose) = (0usize, 0usize, None);
    while Instant::now() < deadline {
        if virtual_camera.publish(&frame, width, height).is_err() {
            return (false, "could not publish to the virtual camera".into());
        }
        while let Ok(event) = service.events.try_recv() {
            match event {
                Event::Fired { gesture: Gesture::OpenPalm, summary } => {
                    let ok = !summary.contains("failed");
                    return (ok, format!("hand recognised -> {summary}"));
                }
                Event::Error(text) => last_error = Some(text),
                Event::Pose(pose) => {
                    rounds += 1;
                    last_pose = pose;
                }
                Event::Frame(_) => frames += 1,
                _ => {}
            }
        }
        std::thread::sleep(FRAME_INTERVAL);
    }
    let seen = format!("{frames} frames, {rounds} detection rounds, last pose {last_pose:?}");
    (false, format!("{} ({seen})", last_error.unwrap_or_else(|| "no gesture fired in time".into())))
}
