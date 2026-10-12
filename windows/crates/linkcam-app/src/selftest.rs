//! `linkcam selftest`: drives the same camera service the window uses and
//! reports PASS/FAIL per feature. Meant for checking a new machine/camera.

use crate::service::{Event, Request, Service};
use linkcam_core::config::Config;
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

    let video_path: PathBuf = std::env::temp_dir().join("linkcam-selftest.mp4");
    service.send(Request::StartRecording(video_path.clone()));
    std::thread::sleep(Duration::from_secs(RECORD_SECONDS));
    service.send(Request::StopRecording);
    let recorded = wait_for(&service, |e| match e {
        Event::Notice(text) if text.starts_with("Saved") => Some(()),
        _ => None,
    });
    let size = std::fs::metadata(&video_path).map_or(0, |m| m.len());
    ok &= report("recording", recorded.is_some() && size > 0, &format!("{} KB", size / 1024));

    report("overall", ok, "");
    ok
}
