//! The controller window: preview, side panel, toolbar and gesture settings.

mod gestures_dialog;
mod side_panel;
mod theme;
mod toolbar;
mod widgets;

use crate::service::{Event, Request, Service};
use crate::store;
use eframe::egui::{self, Align2, Color32, ColorImage, FontId, TextureHandle, TextureOptions, Vec2};
use linkcam_core::config::Config;
use linkcam_core::gesture::Gesture;
use linkcam_core::{CameraState, ControlValue, ImageControl, VideoFormat, VideoMode};
use std::time::{Duration, Instant};

/// How long transient messages stay in the status line.
const NOTICE_TTL: Duration = Duration::from_secs(5);
const DEFAULT_ASPECT: f32 = 16.0 / 9.0;

pub struct LinkCamApp {
    service: Service,
    config: Config,
    connected: bool,
    camera_name: String,
    state: Option<CameraState>,
    image_controls: Vec<(ImageControl, ControlValue)>,
    texture: Option<TextureHandle>,
    frame_size: (usize, usize),
    formats: Vec<VideoFormat>,
    current_format: Option<VideoFormat>,
    previewing: bool,
    recording_since: Option<Instant>,
    virtual_camera_on: bool,
    microphones: Vec<String>,
    virtual_camera_installed: bool,
    pose: Option<Gesture>,
    last_fired: Option<(Instant, String)>,
    notice: Option<(Instant, String, bool)>,
    settings_open: bool,
}

impl LinkCamApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let config = store::load_config();
        let repaint_ctx = cc.egui_ctx.clone();
        let service = Service::spawn(config.clone(), move || repaint_ctx.request_repaint());
        Self {
            service,
            config,
            connected: false,
            camera_name: String::new(),
            state: None,
            image_controls: Vec::new(),
            texture: None,
            frame_size: (0, 0),
            formats: Vec::new(),
            current_format: None,
            previewing: true,
            recording_since: None,
            virtual_camera_on: false,
            microphones: linkcam_win::audio::list_microphones(),
            virtual_camera_installed: linkcam_win::vcam::is_registered(),
            pose: None,
            last_fired: None,
            notice: None,
            settings_open: false,
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.service.events.try_recv() {
            match event {
                Event::Connected { name } => {
                    self.connected = true;
                    self.camera_name = name;
                }
                Event::Disconnected => {
                    self.connected = false;
                    self.texture = None;
                }
                Event::Frame(frame) => {
                    let image = ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
                    self.frame_size = (frame.width, frame.height);
                    match self.texture.as_mut() {
                        Some(texture) => texture.set(image, TextureOptions::LINEAR),
                        None => {
                            self.texture = Some(ctx.load_texture("preview", image, TextureOptions::LINEAR))
                        }
                    }
                }
                Event::State(state) => self.state = Some(state),
                Event::Formats { available, current } => {
                    self.formats = available;
                    self.current_format = current;
                }
                Event::Image(values) => self.image_controls = values,
                Event::Pose(pose) => self.pose = pose,
                Event::VirtualCamera(on) => {
                    self.virtual_camera_on = on;
                    self.virtual_camera_installed = linkcam_win::vcam::is_registered();
                }
                Event::Recording(path) => {
                    self.recording_since = path.map(|_| Instant::now());
                }
                Event::Fired { gesture, summary } => {
                    self.last_fired = Some((Instant::now(), format!("{}: {summary}", gesture.label())));
                }
                Event::Notice(text) => self.notice = Some((Instant::now(), text, false)),
                Event::Error(text) => self.notice = Some((Instant::now(), text, true)),
            }
        }
    }

    fn save_config(&mut self) {
        if let Err(error) = store::save_config(&self.config) {
            self.notice = Some((Instant::now(), format!("Could not save settings: {error}"), true));
        }
    }

    fn set_mode(&mut self, mode: VideoMode) {
        // Pressing the active mode again returns to normal, like the official toggles.
        let active = self.state.and_then(|s| s.mode);
        let target = if active == Some(mode) { VideoMode::Normal } else { mode };
        self.service.send(Request::SetMode(target));
    }

    fn choose_format(&mut self, format: VideoFormat) {
        self.current_format = Some(format);
        self.config.preferred_format = Some(format);
        self.service.send(Request::SetFormat(format));
        self.save_config();
    }

    fn toggle_preview(&mut self) {
        self.previewing = !self.previewing;
        if self.previewing {
            self.service.send(Request::StartPreview);
        } else {
            self.service.send(Request::StopPreview);
            self.texture = None;
        }
    }

    fn draw_preview(&self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        ui.painter().rect_filled(available, 8.0, Color32::BLACK);
        let Some(texture) = self.texture.as_ref().filter(|_| self.previewing) else {
            let (title, hint) = if !self.connected {
                ("Camera not found", "Plug in the Insta360 Link 2C and it will connect automatically.")
            } else {
                ("Preview closed", "You can select Insta360 Link 2C / Virtual Camera in third-party applications to stream video.")
            };
            ui.painter().text(
                available.center() - Vec2::new(0.0, 12.0),
                Align2::CENTER_CENTER,
                title,
                FontId::proportional(20.0),
                theme::TEXT,
            );
            ui.painter().text(
                available.center() + Vec2::new(0.0, 16.0),
                Align2::CENTER_CENTER,
                hint,
                FontId::proportional(13.0),
                theme::MUTED,
            );
            return;
        };
        let (w, h) = (available.width(), available.height());
        let aspect = match self.frame_size {
            (fw, fh) if fw > 0 && fh > 0 => fw as f32 / fh as f32,
            _ => DEFAULT_ASPECT,
        };
        let size = if w / h > aspect { Vec2::new(h * aspect, h) } else { Vec2::new(w, w / aspect) };
        let rect = egui::Rect::from_center_size(available.center(), size);
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        self.draw_overlays(ui, rect);
    }

    /// Gesture feedback drawn on top of the preview.
    fn draw_overlays(&self, ui: &mut egui::Ui, rect: egui::Rect) {
        if self.config.custom_gestures_enabled {
            if let Some(pose) = self.pose {
                ui.painter().text(
                    rect.left_top() + Vec2::new(12.0, 12.0),
                    Align2::LEFT_TOP,
                    format!("\u{270b} {}", pose.label()),
                    FontId::proportional(15.0),
                    theme::ACCENT,
                );
            }
        }
        if let Some((at, text)) = self.last_fired.as_ref().filter(|(at, _)| at.elapsed() < NOTICE_TTL) {
            let _ = at;
            ui.painter().text(
                rect.center_bottom() - Vec2::new(0.0, 14.0),
                Align2::CENTER_BOTTOM,
                text,
                FontId::proportional(15.0),
                theme::ACCENT,
            );
        }
    }

    fn draw_status_line(&mut self, ui: &mut egui::Ui) {
        if let Some((at, text, is_error)) = self.notice.as_ref() {
            if at.elapsed() < NOTICE_TTL {
                let color = if *is_error { theme::DANGER } else { theme::MUTED };
                ui.label(egui::RichText::new(text).color(color).size(12.0));
                return;
            }
        }
        self.notice = None;
    }
}

impl eframe::App for LinkCamApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_events(&ctx);

        egui::Panel::bottom("toolbar")
            .exact_size(112.0)
            .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(egui::Margin::symmetric(16, 8)))
            .show(ui, |ui| self.toolbar(ui));

        egui::Panel::right("side")
            .exact_size(290.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::same(14)))
            .show(ui, |ui| self.side_panel(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(egui::Margin::same(14)))
            .show(ui, |ui| {
                self.draw_preview(ui);
            });

        self.gestures_window(&ctx);
        if self.notice.is_some() || self.last_fired.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}
