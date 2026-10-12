// Release builds are GUI-subsystem; console output is re-attached below for CLI use.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod autostart;
mod cli;
mod selftest;
mod service;
mod store;
mod ui;

use cli::Command;
use linkcam_core::gesture::Gesture;
use linkcam_win::{list_cameras, ComGuard, WinCamera};
use service::{Event, Service};
use std::path::PathBuf;

/// Frames dropped before a snapshot so auto exposure can settle.
const SNAP_WARMUP_FRAMES: usize = 15;
const WINDOW_SIZE: [f32; 2] = [1180.0, 720.0];
const WINDOW_MIN_SIZE: [f32; 2] = [920.0, 580.0];

fn main() {
    attach_parent_console();
    let args: Vec<String> = std::env::args().collect();
    let code = match cli::parse(&args).and_then(run) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    };
    std::process::exit(code);
}

/// A GUI-subsystem exe has no console; attach the parent's so `linkcam status` prints.
fn attach_parent_console() {
    #[cfg(all(not(debug_assertions), windows))]
    unsafe {
        use windows_console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(all(not(debug_assertions), windows))]
mod windows_console {
    pub const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn AttachConsole(process_id: u32) -> i32;
    }
}

/// The window thread must stay free of COM (winit initialises it as STA); the
/// camera service thread and the CLI commands each initialise their own.
fn run(command: Command) -> linkcam_core::Result<()> {
    match command {
        Command::Gui => return run_gui(),
        Command::Gestures => return run_gestures_console(),
        Command::Selftest => {
            return if selftest::run() {
                Ok(())
            } else {
                Err(linkcam_core::CameraError::ReadFailed("selftest failed".into()))
            };
        }
        _ => {}
    }
    let _com = ComGuard::init();
    match command {
        Command::Help => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        Command::List => {
            for camera in list_cameras()? {
                println!("{}  {}", camera.name, camera.device_path);
            }
            Ok(())
        }
        Command::Snap { path, format } => snap(path, format),
        Command::Formats => formats(),
        Command::Autostart(change) => {
            if let Some(enabled) = change {
                autostart::set_enabled(enabled)?;
            }
            println!("autostart: {}", if autostart::is_enabled() { "on" } else { "off" });
            Ok(())
        }
        Command::Record { seconds, path } => record(seconds, path),
        Command::Gui | Command::Gestures | Command::Selftest => Ok(()),
        other => {
            let mut camera = WinCamera::open()?;
            cli::execute(&other, &mut camera, &mut std::io::stdout())
        }
    }
}

fn run_gui() -> linkcam_core::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("LinkCam - Insta360 Link 2C")
            .with_inner_size(WINDOW_SIZE)
            .with_min_inner_size(WINDOW_MIN_SIZE),
        ..Default::default()
    };
    eframe::run_native("LinkCam", options, Box::new(|cc| Ok(Box::new(ui::LinkCamApp::new(cc)))))
        .map_err(|e| linkcam_core::CameraError::ReadFailed(e.to_string()))
}

/// Headless gesture engine: prints what it sees and runs the bound actions.
fn run_gestures_console() -> linkcam_core::Result<()> {
    let mut config = store::load_config();
    config.custom_gestures_enabled = true;
    let service = Service::spawn(config, || {});
    println!("Gesture engine running. Ctrl+C to stop.");
    let mut last_pose: Option<Gesture> = None;
    while let Ok(event) = service.events.recv() {
        match event {
            Event::Connected { name } => println!("camera: {name}"),
            Event::Pose(pose) if pose != last_pose => {
                println!("pose: {}", pose.map_or("-", Gesture::label));
                last_pose = pose;
            }
            Event::Fired { gesture, summary } => println!("FIRED {} -> {summary}", gesture.label()),
            Event::Error(text) => eprintln!("error: {text}"),
            _ => {}
        }
    }
    Ok(())
}

fn snap(path: Option<String>, format: Option<String>) -> linkcam_core::Result<()> {
    let target: PathBuf = path.map_or_else(|| PathBuf::from("linkcam-snap.png"), PathBuf::from);
    let mut camera = WinCamera::open()?;
    camera.ensure_streaming()?;
    if let Some(label) = format {
        let wanted = camera
            .available_formats()
            .iter()
            .copied()
            .find(|f| f.label().eq_ignore_ascii_case(&label))
            .ok_or_else(|| {
                linkcam_core::CameraError::InvalidArgument(format!(
                    "unknown format `{label}`; see `linkcam formats`"
                ))
            })?;
        camera.set_format(wanted)?;
        camera.ensure_streaming()?;
    }
    // Let auto exposure settle before the frame that is kept.
    for _ in 0..SNAP_WARMUP_FRAMES {
        camera.next_frame()?;
    }
    let frame = camera.full_resolution_frame()?;
    linkcam_win::preview::save_png(&frame, &target)?;
    println!("saved {} ({}x{})", target.display(), frame.width, frame.height);
    Ok(())
}

fn formats() -> linkcam_core::Result<()> {
    let mut camera = WinCamera::open()?;
    camera.ensure_streaming()?;
    let current = camera.current_format();
    for format in camera.available_formats() {
        let marker = if Some(*format) == current { "*" } else { " " };
        println!("{marker} {:<9} {}x{} @ {} fps", format.label(), format.width, format.height, format.fps);
    }
    Ok(())
}

fn record(seconds: u64, path: Option<String>) -> linkcam_core::Result<()> {
    let target: PathBuf = path.map_or_else(|| PathBuf::from("linkcam-recording.mp4"), PathBuf::from);
    let mut camera = WinCamera::open()?;
    camera.ensure_streaming()?;
    let fps = camera.current_format().map_or(30, |f| f.fps);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut recorder = None;
    let mut frames = 0u32;
    while std::time::Instant::now() < deadline {
        let Some(frame) = camera.next_frame()? else { continue };
        let active = match recorder.as_mut() {
            Some(active) => active,
            None => recorder.insert(linkcam_win::record::Recorder::create(
                &target,
                frame.width,
                frame.height,
                fps,
            )?),
        };
        active.write_frame(&frame)?;
        frames += 1;
    }
    if let Some(recorder) = recorder {
        recorder.finish()?;
    }
    println!("recorded {frames} frames to {}", target.display());
    Ok(())
}
