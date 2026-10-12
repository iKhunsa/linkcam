//! `linkcam vcam ...`: install, run and self-test the virtual camera.

use linkcam_core::vcam::{CAMERA_NAME, HEIGHT, WIDTH};
use linkcam_core::{CameraError, Result};
use linkcam_win::preview::PreviewCapture;
use linkcam_win::vcam::{self, VirtualCamera};
use linkcam_win::{ComGuard, WinCamera};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Substring that identifies the camera in the capture device list.
const DEVICE_MATCH: &str = "LinkCam Virtual";
const TEST_SECONDS: u64 = 16;
const CONSUME_SECONDS: u64 = 4;
const ENUMERATION_WAIT: Duration = Duration::from_secs(10);
const FRAME_INTERVAL: Duration = Duration::from_millis(33);
/// Allowed per-channel error between the published and the received colour.
const COLOUR_TOLERANCE: i32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install,
    Uninstall,
    Status,
    Run(u64),
    Test,
}

/// Stages the DLL where the Frame Server account can read it.
fn staged_dll() -> Result<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| CameraError::ReadFailed(e.to_string()))?;
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    vcam::stage_dll(&dir)
}

pub fn run(action: Action) -> Result<()> {
    match action {
        Action::Install => {
            println!(
                "Registering {CAMERA_NAME} system-wide: approve the Windows administrator prompt (once)."
            );
            vcam::register(&staged_dll()?)?;
            println!("registered");
            Ok(())
        }
        Action::Uninstall => {
            vcam::unregister()?;
            println!("unregistered");
            Ok(())
        }
        Action::Status => {
            println!("registered: {}", vcam::is_registered());
            Ok(())
        }
        Action::Run(seconds) => relay_camera(seconds),
        Action::Test => self_test(),
    }
}

/// Publishes the physical camera into the virtual one for `seconds`.
fn relay_camera(seconds: u64) -> Result<()> {
    let _com = ComGuard::init();
    let mut camera = WinCamera::open()?;
    let mut virtual_camera = VirtualCamera::start()?;
    println!("{CAMERA_NAME} is live for {seconds} s - select it in any app");
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if let Some(frame) = camera.next_frame()? {
            virtual_camera.publish(&frame.rgba, frame.width, frame.height)?;
        }
    }
    Ok(())
}

/// Left half red, right half blue, in RGBA.
fn test_card() -> Vec<u8> {
    let mut rgba = vec![0u8; WIDTH * HEIGHT * 4];
    for pixel_index in 0..WIDTH * HEIGHT {
        let x = pixel_index % WIDTH;
        let colour = if x < WIDTH / 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
        rgba[pixel_index * 4..pixel_index * 4 + 4].copy_from_slice(&colour);
    }
    rgba
}

struct Seen {
    frames: usize,
    left: (u8, u8, u8),
    right: (u8, u8, u8),
}

/// Reads the virtual camera through the normal capture path, like any other app.
fn consume() -> Result<Seen> {
    let _com = ComGuard::init();
    let deadline = Instant::now() + ENUMERATION_WAIT;
    let mut capture = loop {
        match PreviewCapture::open(DEVICE_MATCH, None) {
            Ok(capture) => break capture,
            Err(error) if Instant::now() > deadline => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    };
    let mut frames = 0;
    let mut last = None;
    let until = Instant::now() + Duration::from_secs(CONSUME_SECONDS);
    while Instant::now() < until {
        if let Some(frame) = capture.next_frame()? {
            frames += 1;
            last = Some(frame);
        }
    }
    let frame = last.ok_or(CameraError::ReadFailed("the virtual camera delivered no frames".into()))?;
    let pixel = |percent_x: usize| {
        let at = ((frame.height / 2) * frame.width + frame.width * percent_x / 100) * 4;
        (frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2])
    };
    Ok(Seen { frames, left: pixel(25), right: pixel(75) })
}

fn near(actual: (u8, u8, u8), expected: (u8, u8, u8)) -> bool {
    let close = |a: u8, b: u8| (a as i32 - b as i32).abs() < COLOUR_TOLERANCE;
    close(actual.0, expected.0) && close(actual.1, expected.1) && close(actual.2, expected.2)
}

/// Starts the virtual camera, publishes a test card and reads it back.
fn self_test() -> Result<()> {
    staged_dll()?;
    let _com = ComGuard::init();
    let mut virtual_camera = VirtualCamera::start()?;
    let card = test_card();

    let consumer = std::thread::spawn(consume);
    let until = Instant::now() + Duration::from_secs(TEST_SECONDS);
    while Instant::now() < until && !consumer.is_finished() {
        virtual_camera.publish(&card, WIDTH, HEIGHT)?;
        std::thread::sleep(FRAME_INTERVAL);
    }
    let seen = consumer.join().map_err(|_| CameraError::ReadFailed("consumer thread panicked".into()))??;

    let left_ok = near(seen.left, (255, 0, 0));
    let right_ok = near(seen.right, (0, 0, 255));
    println!("frames received through {CAMERA_NAME}: {}", seen.frames);
    println!("left  pixel {:?} expected red  -> {}", seen.left, if left_ok { "ok" } else { "MISMATCH" });
    println!("right pixel {:?} expected blue -> {}", seen.right, if right_ok { "ok" } else { "MISMATCH" });
    if left_ok && right_ok && seen.frames > 0 {
        Ok(())
    } else {
        Err(CameraError::ReadFailed("virtual camera output did not match the test card".into()))
    }
}
