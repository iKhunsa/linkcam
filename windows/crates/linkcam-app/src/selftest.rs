//! `linkcam selftest`: drives the same camera service the window uses and
//! reports PASS/FAIL per feature. Meant for checking a new machine/camera.

use crate::service::{Event, Request, Service};
use linkcam_core::config::{AudioConfig, Config, EffectsConfig};
use linkcam_core::{VideoFormat, VideoMode, ZoomUnits};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const RECORD_SECONDS: u64 = 3;
const TEST_FORMAT: VideoFormat = VideoFormat::new(1920, 1080, 30);

/// Consumes events until `matcher` returns a value or the timeout expires.
fn wait_for<T>(service: &Service, mut matcher: impl FnMut(&Event) -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + STEP_TIMEOUT;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match service.events.recv_timeout(remaining) {
            Ok(event) => {
                if let Some(value) = matcher(&event) {
                    return Some(value);
                }
            }
            Err(_) => return None,
        }
    }
    None
}

fn report(name: &str, passed: bool, detail: &str) -> bool {
    println!(
        "[{}] {name}{}",
        if passed { "PASS" } else { "FAIL" },
        if detail.is_empty() { String::new() } else { format!(" - {detail}") }
    );
    passed
}

pub fn run() -> bool {
    let service = Service::spawn(Config::default(), || {});
    let mut ok = true;

    let connected = wait_for(&service, |e| match e {
        Event::Connected { name } => Some(name.clone()),
        _ => None,
    });
    ok &= report("camera found", connected.is_some(), connected.as_deref().unwrap_or("not found"));
    if connected.is_none() {
        return false;
    }

    let frame = wait_for(&service, |e| match e {
        Event::Frame(f) => Some((f.width, f.height)),
        _ => None,
    });
    ok &=
        report("preview frames", frame.is_some(), &frame.map_or(String::new(), |(w, h)| format!("{w}x{h}")));

    for mode in [VideoMode::DeskView, VideoMode::Normal] {
        service.send(Request::SetMode(mode));
        let applied = wait_for(&service, |e| match e {
            Event::State(s) if s.mode == Some(mode) => Some(()),
            Event::Error(_) => None,
            _ => None,
        });
        ok &= report(&format!("mode {}", mode.label()), applied.is_some(), "");
    }

    for target in [200u16, 100] {
        service.send(Request::SetZoom(ZoomUnits::from_raw(target)));
        let applied = wait_for(&service, |e| match e {
            Event::State(s) if s.zoom.raw() == target => Some(()),
            _ => None,
        });
        ok &= report(&format!("zoom {:.1}x", target as f64 / 100.0), applied.is_some(), "");
    }

    service.send(Request::SetFormat(TEST_FORMAT));
    let format = wait_for(&service, |e| match e {
        Event::Formats { current: Some(f), .. } if *f == TEST_FORMAT => Some(*f),
        _ => None,
    });
    ok &= report("resolution 1080p30", format.is_some(), "");

    let snapshot_path: PathBuf = std::env::temp_dir().join("linkcam-selftest.png");
    service.send(Request::Snapshot(snapshot_path.clone()));
    let saved = wait_for(&service, |e| match e {
        Event::Notice(text) if text.starts_with("Saved") => Some(text.clone()),
        Event::Notice(text) => Some(format!("FAILED: {text}")),
        _ => None,
    });
    ok &= report(
        "snapshot",
        saved.as_ref().is_some_and(|t| !t.starts_with("FAILED")),
        saved.as_deref().unwrap_or("timeout"),
    );

    ok &= record_and_check(&service, "recording (video + microphone)", "linkcam-selftest.mp4");

    service.send(Request::SetAudio(AudioConfig { noise_reduction: true, ..AudioConfig::default() }));
    ok &= record_and_check(&service, "recording (noise reduction)", "linkcam-selftest-nr.mp4");
    service.send(Request::SetAudio(AudioConfig::default()));

    // Software effects: the overlay text must show up in the delivered frames and
    // the segmentation model must load and run without errors.
    let effects = EffectsConfig {
        blur_background: true,
        overlay_enabled: true,
        overlay_text: "LinkCam".into(),
        ..EffectsConfig::default()
    };
    service.send(Request::SetEffects(effects));
    let mut problem: Option<String> = None;
    let overlay_seen = wait_for(&service, |event| match event {
        Event::Error(text) => {
            problem = Some(text.clone());
            Some(false)
        }
        Event::Frame(frame) => has_overlay_in_corner(frame).then_some(true),
        _ => None,
    });
    ok &= report(
        "effects (blur + text overlay)",
        overlay_seen == Some(true) && problem.is_none(),
        problem.as_deref().unwrap_or(""),
    );
    service.send(Request::SetEffects(EffectsConfig::default()));

    // Only one client may stream from the camera at a time.
    drop(service);
    ok &= gesture_engine_on_live_frames();

    report("overall", ok, "");
    ok
}

/// Runs the gesture engine on the camera's real frames and checks it keeps
/// producing detection rounds without errors (a hand is not required).
fn gesture_engine_on_live_frames() -> bool {
    const ROUNDS_REQUIRED: usize = 5;
    let config = Config { custom_gestures_enabled: true, ..Config::default() };
    let service = Service::spawn(config, || {});
    let mut rounds = 0;
    let mut problem: Option<String> = None;
    let finished = wait_for(&service, |event| {
        match event {
            Event::Pose(_) => rounds += 1,
            Event::Error(text) => problem = Some(text.clone()),
            _ => {}
        }
        (rounds >= ROUNDS_REQUIRED || problem.is_some()).then_some(())
    });
    let passed = finished.is_some() && problem.is_none();
    let detail = problem.unwrap_or_else(|| format!("{rounds} detection rounds on live video"));
    report("gesture engine (live)", passed, &detail)
}

/// True when the bottom-left corner holds both overlay colours (white fill, black outline).
fn has_overlay_in_corner(frame: &linkcam_win::preview::Frame) -> bool {
    const CORNER: usize = 40;
    let mut white = false;
    let mut black = false;
    for y in frame.height.saturating_sub(CORNER)..frame.height {
        for x in 0..CORNER.min(frame.width) {
            let px = &frame.rgba[(y * frame.width + x) * 4..(y * frame.width + x) * 4 + 3];
            white |= px == [255, 255, 255];
            black |= px == [0, 0, 0];
        }
    }
    white && black
}

/// Records a few seconds and checks the file holds both a video and an audio track.
fn record_and_check(service: &Service, name: &str, file: &str) -> bool {
    let path: PathBuf = std::env::temp_dir().join(file);
    service.send(Request::StartRecording(path.clone()));
    std::thread::sleep(Duration::from_secs(RECORD_SECONDS));
    service.send(Request::StopRecording);
    let saved = wait_for(service, |event| match event {
        Event::Notice(text) if text.starts_with("Saved") => Some(()),
        _ => None,
    });
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    let tracks = linkcam_win::record::describe_file(&path).unwrap_or((false, false));
    report(
        name,
        saved.is_some() && size > 0 && tracks == (true, true),
        &format!("{} KB, video={}, audio={}", size / 1024, tracks.0, tracks.1),
    )
}
