//! Camera service: one thread owns the COM camera, streams frames, applies
//! requests from the UI and runs the gesture engine. The UI only exchanges
//! messages with it, so COM never crosses threads.

use crate::store;
use linkcam_core::config::Config;
use linkcam_core::gesture::{Action, Binding, CameraCommand, Gesture};
use linkcam_core::{
    CameraDevice, CameraState, ControlValue, ImageControl, Result as CamResult, VideoFormat, VideoMode,
    ZoomUnits,
};
use linkcam_vision::{classify, Dispatcher, HandModels, ImageView};
use linkcam_win::preview::Frame;
use linkcam_win::record::Recorder;
use linkcam_win::{keys, ComGuard, WinCamera};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Detection runs on every Nth frame while a hand is in view (~10 fps at 30 fps video)...
const GESTURE_STRIDE_ACTIVE: u32 = 3;
/// ...and less often while idle, which is what keeps the CPU low.
const GESTURE_STRIDE_IDLE: u32 = 6;
/// How long after the last hand sighting the faster rate is kept.
const HAND_RECENT: Duration = Duration::from_millis(1500);
/// Width the frame is reduced to before hand detection.
const GESTURE_INPUT_WIDTH: usize = 480;
/// Pause between attempts to find the camera.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// Idle sleep when the preview is stopped.
const IDLE_DELAY: Duration = Duration::from_millis(40);
/// Zoom step used by the `ZoomBy` camera command.
const ZOOM_STEP_UNITS: i32 = 100;

pub enum Request {
    StartPreview,
    StopPreview,
    SetZoom(ZoomUnits),
    SetMode(VideoMode),
    SetFormat(VideoFormat),
    SetImage { control: ImageControl, value: i32, auto: bool },
    Snapshot(PathBuf),
    StartRecording(PathBuf),
    StopRecording,
    GesturesEnabled(bool),
    Bindings(Vec<Binding>),
    Shutdown,
}

pub enum Event {
    Connected {
        name: String,
    },
    Disconnected,
    Frame(Arc<Frame>),
    State(CameraState),
    /// Stream formats the camera offers and the one in use.
    Formats {
        available: Vec<VideoFormat>,
        current: Option<VideoFormat>,
    },
    Image(Vec<(ImageControl, ControlValue)>),
    /// What the gesture engine currently sees (`None` = no hand).
    Pose(Option<Gesture>),
    Fired {
        gesture: Gesture,
        summary: String,
    },
    /// `Some(path)` while recording to `path`, `None` when stopped.
    Recording(Option<PathBuf>),
    Notice(String),
    Error(String),
}

pub struct Service {
    requests: Sender<Request>,
    pub events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Service {
    /// Starts the camera thread. `repaint` wakes the UI when events arrive.
    pub fn spawn(config: Config, repaint: impl Fn() + Send + 'static) -> Self {
        let (request_tx, request_rx) = channel();
        let (event_tx, event_rx) = channel();
        let thread = thread::Builder::new()
            .name("camera".into())
            .spawn(move || Worker::new(config, request_rx, event_tx, Box::new(repaint)).run())
            .ok();
        Self { requests: request_tx, events: event_rx, thread }
    }

    pub fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Worker {
    requests: Receiver<Request>,
    events: Sender<Event>,
    repaint: Box<dyn Fn() + Send>,
    config: Config,
    previewing: bool,
    gestures_enabled: bool,
    engine: Option<Engine>,
    last_frame: Option<Arc<Frame>>,
    frame_counter: u32,
    formats_announced: bool,
    last_hand_seen: Option<Instant>,
    recorder: Option<(Recorder, PathBuf)>,
}

/// Hand detection + dispatching, created on demand.
struct Engine {
    models: HandModels,
    dispatcher: Dispatcher,
    started: Instant,
}

impl Worker {
    fn new(
        config: Config,
        requests: Receiver<Request>,
        events: Sender<Event>,
        repaint: Box<dyn Fn() + Send>,
    ) -> Self {
        let gestures_enabled = config.custom_gestures_enabled;
        Self {
            requests,
            events,
            repaint,
            config,
            previewing: true,
            gestures_enabled,
            engine: None,
            last_frame: None,
            frame_counter: 0,
            formats_announced: false,
            last_hand_seen: None,
            recorder: None,
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
        (self.repaint)();
    }

    fn run(mut self) {
        let _com = ComGuard::init();
        loop {
            let mut camera = match WinCamera::open() {
                Ok(camera) => camera,
                Err(_) => {
                    self.emit(Event::Disconnected);
                    if !self.wait_while_serving_requests(RECONNECT_DELAY) {
                        return;
                    }
                    continue;
                }
            };
            self.emit(Event::Connected { name: camera.info().name.clone() });
            if let Some(format) = self.config.preferred_format {
                let _ = camera.set_format(format);
            }
            self.formats_announced = false;
            self.publish_state(&mut camera);
            if !self.serve(&mut camera) {
                return;
            }
        }
    }

    /// Sleeps for `duration` but still honours `Shutdown`. Returns false on shutdown.
    fn wait_while_serving_requests(&mut self, duration: Duration) -> bool {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            match self.requests.try_recv() {
                Ok(Request::Shutdown) | Err(TryRecvError::Disconnected) => return false,
                Ok(_) | Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(50)),
            }
        }
        true
    }

    /// Main loop while a camera is open. Returns false when the service must stop.
    fn serve(&mut self, camera: &mut WinCamera) -> bool {
        loop {
            loop {
                match self.requests.try_recv() {
                    Ok(Request::Shutdown) | Err(TryRecvError::Disconnected) => return false,
                    Ok(request) => self.handle(request, camera),
                    Err(TryRecvError::Empty) => break,
                }
            }

            if !self.previewing {
                camera.stop_streaming();
                thread::sleep(IDLE_DELAY);
                continue;
            }
            match camera.next_frame() {
                Ok(Some(frame)) => self.on_frame(frame, camera),
                Ok(None) => {}
                Err(error) => {
                    self.emit(Event::Error(error.to_string()));
                    camera.stop_streaming();
                    // Reopen the whole device: the camera may have been unplugged.
                    return self.wait_while_serving_requests(RECONNECT_DELAY);
                }
            }
        }
    }

    fn handle(&mut self, request: Request, camera: &mut WinCamera) {
        match request {
            Request::StartPreview => self.previewing = true,
            Request::StopPreview => {
                self.stop_recording();
                self.previewing = false;
                self.last_frame = None;
            }
            Request::SetZoom(zoom) => self.report(camera.set_zoom(zoom), Event::State),
            Request::SetMode(mode) => self.report(camera.set_mode(mode), Event::State),
            Request::SetFormat(format) => {
                self.config.preferred_format = Some(format);
                match camera.set_format(format) {
                    Ok(()) => {
                        self.formats_announced = false;
                        self.publish_formats(camera);
                    }
                    Err(error) => self.emit(Event::Error(error.to_string())),
                }
            }
            Request::SetImage { control, value, auto } => {
                match camera.set_image_control(control, value, auto) {
                    Ok(()) => self.publish_image_controls(camera),
                    Err(error) => self.emit(Event::Error(error.to_string())),
                }
            }
            Request::Snapshot(path) => self.snapshot(path, camera),
            Request::StartRecording(path) => self.start_recording(path, camera),
            Request::StopRecording => self.stop_recording(),
            Request::GesturesEnabled(on) => {
                self.gestures_enabled = on;
                self.config.custom_gestures_enabled = on;
                if !on {
                    self.emit(Event::Pose(None));
                }
            }
            Request::Bindings(bindings) => {
                self.config.bindings = bindings;
                if let Some(engine) = self.engine.as_mut() {
                    engine.dispatcher = Dispatcher::new(&self.config.bindings);
                }
            }
            Request::Shutdown => {}
        }
    }

    fn report(&self, result: CamResult<CameraState>, make: fn(CameraState) -> Event) {
        match result {
            Ok(state) => self.emit(make(state)),
            Err(error) => self.emit(Event::Error(error.to_string())),
        }
    }

    fn publish_state(&mut self, camera: &mut WinCamera) {
        self.report(camera.read_state(), Event::State);
        self.publish_image_controls(camera);
    }

    fn publish_image_controls(&self, camera: &mut WinCamera) {
        let values: Vec<_> = ImageControl::ALL
            .into_iter()
            .filter_map(|c| camera.image_control(c).ok().map(|v| (c, v)))
            .collect();
        self.emit(Event::Image(values));
    }

    /// Saves a full-resolution frame (not the decimated preview).
    fn snapshot(&self, path: PathBuf, camera: &mut WinCamera) {
        let message = match camera.full_resolution_frame() {
            Ok(frame) => match linkcam_win::preview::save_png(&frame, &path) {
                Ok(()) => format!("Saved {} ({}x{})", path.display(), frame.width, frame.height),
                Err(error) => format!("Snapshot failed: {error}"),
            },
            Err(error) => format!("Snapshot failed: {error}"),
        };
        self.emit(Event::Notice(message));
    }

    fn start_recording(&mut self, path: PathBuf, camera: &WinCamera) {
        if self.recorder.is_some() {
            return;
        }
        let Some(frame) = self.last_frame.clone() else {
            self.emit(Event::Error("Start the preview before recording".into()));
            return;
        };
        let fps = camera.current_format().map_or(30, |f| f.fps);
        match Recorder::create(&path, frame.width, frame.height, fps) {
            Ok(recorder) => {
                self.emit(Event::Recording(Some(path.clone())));
                self.recorder = Some((recorder, path));
            }
            Err(error) => self.emit(Event::Error(format!("Could not start recording: {error}"))),
        }
    }

    fn stop_recording(&mut self) {
        let Some((recorder, path)) = self.recorder.take() else { return };
        let message = match recorder.finish() {
            Ok(()) => format!("Saved {}", path.display()),
            Err(error) => format!("Recording failed: {error}"),
        };
        self.emit(Event::Recording(None));
        self.emit(Event::Notice(message));
    }

    fn publish_formats(&mut self, camera: &WinCamera) {
        let available = camera.available_formats().to_vec();
        if available.is_empty() {
            return;
        }
        self.formats_announced = true;
        self.emit(Event::Formats { available, current: camera.current_format() });
    }

    fn on_frame(&mut self, frame: Frame, camera: &mut WinCamera) {
        if !self.formats_announced {
            self.publish_formats(camera);
        }
        let frame = Arc::new(frame);
        self.last_frame = Some(frame.clone());
        if let Some((recorder, _)) = self.recorder.as_mut() {
            if let Err(error) = recorder.write_frame(&frame) {
                self.emit(Event::Error(format!("Recording stopped: {error}")));
                self.stop_recording();
            }
        }
        self.emit(Event::Frame(frame.clone()));

        self.frame_counter = self.frame_counter.wrapping_add(1);
        if self.gestures_enabled && self.frame_counter.is_multiple_of(self.gesture_stride()) {
            self.run_gestures(&frame, camera);
        }
    }

    fn gesture_stride(&self) -> u32 {
        match self.last_hand_seen {
            Some(at) if at.elapsed() < HAND_RECENT => GESTURE_STRIDE_ACTIVE,
            _ => GESTURE_STRIDE_IDLE,
        }
    }

    fn run_gestures(&mut self, frame: &Frame, camera: &mut WinCamera) {
        if self.engine.is_none() && !self.load_engine() {
            self.gestures_enabled = false;
            return;
        }
        let small = frame.downscaled(GESTURE_INPUT_WIDTH);
        let view = ImageView { width: small.width, height: small.height, rgba: &small.rgba };

        let Some(engine) = self.engine.as_mut() else { return };
        let hand = match engine.models.detect(&view) {
            Ok(hand) => hand,
            Err(error) => {
                let message = format!("Gesture engine error: {error}");
                self.gestures_enabled = false;
                self.emit(Event::Error(message));
                return;
            }
        };
        if hand.is_some() {
            self.last_hand_seen = Some(Instant::now());
        }
        let pose = hand.as_ref().and_then(classify);
        let wrist = hand.as_ref().map(|h| h.wrist());
        let now_ms = engine.started.elapsed().as_millis() as u64;
        let fired = engine.dispatcher.update(now_ms, pose, wrist);

        self.emit(Event::Pose(pose));
        for item in fired {
            let summary = describe(&item.action);
            let outcome = perform(&item.action, camera);
            let text = match outcome {
                Ok(()) => summary,
                Err(error) => format!("{summary} failed: {error}"),
            };
            self.emit(Event::Fired { gesture: item.gesture, summary: text });
            self.publish_state(camera);
        }
    }

    fn load_engine(&mut self) -> bool {
        let Some(dir) = store::model_dir(&self.config.model_dir) else {
            self.emit(Event::Error("Gesture models not found (models/*.onnx next to linkcam.exe)".into()));
            return false;
        };
        match HandModels::load(&dir) {
            Ok(models) => {
                self.engine = Some(Engine {
                    models,
                    dispatcher: Dispatcher::new(&self.config.bindings),
                    started: Instant::now(),
                });
                true
            }
            Err(error) => {
                self.emit(Event::Error(format!("Could not load gesture models: {error}")));
                false
            }
        }
    }
}

fn describe(action: &Action) -> String {
    match action {
        Action::Keys { keys } => format!("Keys {}", keys.join("+")),
        Action::Launch { target } => format!("Launch {target}"),
        Action::Camera(CameraCommand::SetMode { mode }) => format!("Mode {}", mode.label()),
        Action::Camera(CameraCommand::ToggleMode { mode }) => format!("Toggle {}", mode.label()),
        Action::Camera(CameraCommand::ZoomBy { percent }) => format!("Zoom {percent:+}%"),
        Action::Camera(CameraCommand::Recenter) => "Recenter".into(),
    }
}

/// Executes an action fired by a gesture.
fn perform(action: &Action, camera: &mut WinCamera) -> CamResult<()> {
    match action {
        Action::Keys { keys } => keys::press_chord(keys),
        Action::Launch { target } => std::process::Command::new("cmd")
            .args(["/C", "start", "", target])
            .spawn()
            .map(|_| ())
            .map_err(|e| linkcam_core::CameraError::WriteFailed(e.to_string())),
        Action::Camera(command) => perform_camera(*command, camera),
    }
}

fn perform_camera(command: CameraCommand, camera: &mut WinCamera) -> CamResult<()> {
    match command {
        CameraCommand::SetMode { mode } => camera.set_mode(mode).map(|_| ()),
        CameraCommand::ToggleMode { mode } => {
            let target = if camera.read_state()?.mode == Some(mode) { VideoMode::Normal } else { mode };
            camera.set_mode(target).map(|_| ())
        }
        CameraCommand::ZoomBy { percent } => {
            let limits = camera.limits();
            let current = camera.read_state()?.zoom.raw() as i32;
            let step = ZOOM_STEP_UNITS * percent as i32 / 100;
            let target = ZoomUnits::from_raw_clamped((current + step) as i64, limits.zoom);
            camera.set_zoom(target).map(|_| ())
        }
        CameraCommand::Recenter => {
            camera.set_zoom(ZoomUnits::ONE_X)?;
            camera.set_mode(VideoMode::Normal).map(|_| ())
        }
    }
}
