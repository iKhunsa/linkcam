//! Command-line interface. Parsing and execution are separate and run
//! against `CameraDevice`, so everything is testable with `FakeCamera`.

use linkcam_core::units::{parse_angle, parse_zoom};
use linkcam_core::{CameraDevice, CameraError, CameraState, ImageControl, Result, VideoMode};
use std::io::Write;

pub const USAGE: &str = "\
linkcam - Insta360 Link 2C controller for Windows

USAGE:
  linkcam                         open the controller window
  linkcam status [--json]         zoom and mode
  linkcam zoom <1..4|1.5x|150>    set zoom
  linkcam pan|tilt <deg>          gimbal models only (ignored by the Link 2C)
  linkcam mode <name>             normal | whiteboard | overhead | deskview | auto-framing
  linkcam image <control> <v|auto>  brightness|contrast|saturation|sharpness|white-balance|exposure
  linkcam record <seconds> [f.mp4] record the preview to an H.264 file
  linkcam formats                 list stream resolutions (4K30, 1080p30, ...)
  linkcam snap [file.png] [4K30]  save a full-resolution frame, optionally at a format
  linkcam list                    list capture devices
  linkcam gestures                run the gesture engine in the console
  linkcam autostart on|off|status start LinkCam with Windows
  linkcam vcam install|uninstall|status|run <s>|test   virtual camera (Windows 11)
  linkcam selftest [hand]         exercise mode/zoom/resolution/snapshot/recording
  linkcam help";

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Gui,
    Status {
        json: bool,
    },
    Zoom(String),
    Pan(String),
    Tilt(String),
    Mode(VideoMode),
    Image {
        control: ImageControl,
        value: ImageValue,
    },
    Snap {
        path: Option<String>,
        format: Option<String>,
    },
    Formats,
    Record {
        seconds: u64,
        path: Option<String>,
    },
    Autostart(Option<bool>),
    /// `true` runs only the hand-through-virtual-camera step.
    Selftest {
        hand_only: bool,
    },
    Vcam(crate::vcam_cli::Action),
    List,
    Gestures,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageValue {
    Auto,
    Manual(i32),
}

fn invalid(message: impl Into<String>) -> CameraError {
    CameraError::InvalidArgument(message.into())
}

fn parse_image_control(name: &str) -> Option<ImageControl> {
    let key = name.to_ascii_lowercase().replace(['-', '_'], "");
    ImageControl::ALL.into_iter().find(|c| c.label().to_ascii_lowercase().replace(' ', "") == key)
}

pub fn parse(args: &[String]) -> Result<Command> {
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let need =
        |what: &str| rest.get(1).map(|s| s.to_string()).ok_or_else(|| invalid(format!("missing {what}")));
    match rest.first().copied() {
        None | Some("gui") => Ok(Command::Gui),
        Some("help" | "-h" | "--help") => Ok(Command::Help),
        Some("status") => Ok(Command::Status { json: rest.contains(&"--json") }),
        Some("zoom") => Ok(Command::Zoom(need("zoom value")?)),
        Some("pan") => Ok(Command::Pan(need("pan degrees")?)),
        Some("tilt") => Ok(Command::Tilt(need("tilt degrees")?)),
        Some("mode") => {
            let name = need("mode name")?;
            VideoMode::from_label(&name)
                .map(Command::Mode)
                .ok_or_else(|| invalid(format!("unknown mode `{name}`")))
        }
        Some("image") => {
            let name = need("control name")?;
            let control =
                parse_image_control(&name).ok_or_else(|| invalid(format!("unknown control `{name}`")))?;
            let value = rest.get(2).ok_or_else(|| invalid("missing value"))?;
            let value = if value.eq_ignore_ascii_case("auto") {
                ImageValue::Auto
            } else {
                ImageValue::Manual(value.parse().map_err(|_| invalid(format!("not a number: `{value}`")))?)
            };
            Ok(Command::Image { control, value })
        }
        Some("snap") => Ok(Command::Snap {
            path: rest.get(1).map(|s| s.to_string()),
            format: rest.get(2).map(|s| s.to_string()),
        }),
        Some("formats") => Ok(Command::Formats),
        Some("selftest") => Ok(Command::Selftest { hand_only: rest.get(1) == Some(&"hand") }),
        Some("vcam") => {
            use crate::vcam_cli::Action;
            match rest.get(1).copied() {
                Some("install") => Ok(Command::Vcam(Action::Install)),
                Some("uninstall") => Ok(Command::Vcam(Action::Uninstall)),
                Some("status") | None => Ok(Command::Vcam(Action::Status)),
                Some("test") => Ok(Command::Vcam(Action::Test)),
                Some("run") => {
                    let seconds = rest.get(2).and_then(|s| s.parse().ok()).unwrap_or(3600);
                    Ok(Command::Vcam(Action::Run(seconds)))
                }
                Some(other) => {
                    Err(invalid(format!("vcam expects install|uninstall|status|run|test, got `{other}`")))
                }
            }
        }
        Some("autostart") => match rest.get(1).copied() {
            Some("on") => Ok(Command::Autostart(Some(true))),
            Some("off") => Ok(Command::Autostart(Some(false))),
            Some("status") | None => Ok(Command::Autostart(None)),
            Some(other) => Err(invalid(format!("autostart expects on|off|status, got `{other}`"))),
        },
        Some("record") => {
            let seconds = need("duration in seconds")?
                .parse::<u64>()
                .map_err(|_| invalid("duration must be a whole number of seconds"))?;
            Ok(Command::Record { seconds, path: rest.get(2).map(|s| s.to_string()) })
        }
        Some("list") => Ok(Command::List),
        Some("gestures") => Ok(Command::Gestures),
        Some(other) => Err(invalid(format!("unknown command `{other}`; try `linkcam help`"))),
    }
}

fn format_state(state: &CameraState, json: bool) -> String {
    let mode = state.mode.map_or("unknown", VideoMode::label);
    if json {
        format!(
            "{{\"zoom\":{:.2},\"pan\":{:.1},\"tilt\":{:.1},\"mode\":\"{mode}\"}}",
            state.zoom.multiplier(),
            state.pan.degrees(),
            state.tilt.degrees()
        )
    } else {
        format!(
            "zoom {:.2}x  pan {:.0}\u{b0}  tilt {:.0}\u{b0}  mode {mode}",
            state.zoom.multiplier(),
            state.pan.degrees(),
            state.tilt.degrees()
        )
    }
}

/// Executes the camera-bound commands. `Gui`, `Snap`, `List`, `Gestures` and
/// `Help` need no `CameraDevice` and are handled by the caller.
pub fn execute(command: &Command, camera: &mut dyn CameraDevice, out: &mut dyn Write) -> Result<()> {
    let limits = camera.limits();
    let say = |out: &mut dyn Write, text: String| {
        writeln!(out, "{text}").map_err(|e| CameraError::WriteFailed(e.to_string()))
    };
    match command {
        Command::Status { json } => {
            let state = camera.read_state()?;
            say(out, format_state(&state, *json))
        }
        Command::Zoom(text) => {
            let state = camera.set_zoom(parse_zoom(text, limits.zoom)?)?;
            say(out, format_state(&state, false))
        }
        Command::Pan(text) => {
            let current = camera.read_state()?;
            let state = camera.set_pan_tilt(parse_angle(text, limits.pan)?, current.tilt)?;
            say(out, format_state(&state, false))
        }
        Command::Tilt(text) => {
            let current = camera.read_state()?;
            let state = camera.set_pan_tilt(current.pan, parse_angle(text, limits.tilt)?)?;
            say(out, format_state(&state, false))
        }
        Command::Mode(mode) => {
            let state = camera.set_mode(*mode)?;
            say(out, format_state(&state, false))
        }
        Command::Image { control, value } => {
            let (number, auto) = match value {
                ImageValue::Auto => (camera.image_control(*control)?.value, true),
                ImageValue::Manual(v) => (*v, false),
            };
            camera.set_image_control(*control, number, auto)?;
            let now = camera.image_control(*control)?;
            say(out, format!("{} = {}{}", control.label(), now.value, if now.auto { " (auto)" } else { "" }))
        }
        Command::Gui
        | Command::Snap { .. }
        | Command::Formats
        | Command::Record { .. }
        | Command::Autostart(_)
        | Command::Selftest { .. }
        | Command::Vcam(_)
        | Command::List
        | Command::Gestures
        | Command::Help => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linkcam_core::FakeCamera;

    fn args(text: &str) -> Vec<String> {
        std::iter::once("linkcam").chain(text.split_whitespace()).map(String::from).collect()
    }

    fn run(text: &str, camera: &mut FakeCamera) -> String {
        let command = parse(&args(text)).unwrap();
        let mut out = Vec::new();
        execute(&command, camera, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn no_arguments_opens_the_gui() {
        assert_eq!(parse(&args("")).unwrap(), Command::Gui);
    }

    #[test]
    fn zoom_is_applied_and_reported() {
        let mut camera = FakeCamera::new();
        assert!(run("zoom 1.5x", &mut camera).contains("zoom 1.50x"));
        assert_eq!(camera.read_state().unwrap().zoom.raw(), 150);
    }

    #[test]
    fn status_json_is_stable() {
        let mut camera = FakeCamera::new();
        assert_eq!(
            run("status --json", &mut camera).trim(),
            r#"{"zoom":1.00,"pan":0.0,"tilt":0.0,"mode":"normal"}"#
        );
    }

    #[test]
    fn mode_names_are_parsed() {
        let mut camera = FakeCamera::new();
        assert!(run("mode deskview", &mut camera).contains("mode deskview"));
    }

    #[test]
    fn image_control_accepts_dashed_names_and_auto() {
        let mut camera = FakeCamera::new();
        assert!(run("image white-balance 4000", &mut camera).contains("White balance = 100"));
        assert!(run("image brightness auto", &mut camera).contains("(auto)"));
    }

    #[test]
    fn snap_accepts_path_and_format() {
        assert_eq!(
            parse(&args("snap a.png 4K30")).unwrap(),
            Command::Snap { path: Some("a.png".into()), format: Some("4K30".into()) }
        );
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(parse(&args("zoom")).is_err());
        assert!(parse(&args("mode banana")).is_err());
        assert!(parse(&args("frobnicate")).is_err());
    }
}
