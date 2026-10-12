//! Right-hand column: device header, scene presets, view, image controls.

use super::widgets::{section_title, status_dot};
use super::{theme, LinkCamApp};
use crate::service::Request;
use eframe::egui::{self, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use linkcam_core::config::ScenePreset;
use linkcam_core::units::ZoomRange;
use linkcam_core::{ImageControl, VideoMode, ZoomUnits};

const PRESET_SLOTS: u8 = 6;
const VIEW_BOX: Vec2 = Vec2::new(176.0, 99.0);
const ZOOM_RANGE: ZoomRange = ZoomRange { min: 100, max: 400 };

impl LinkCamApp {
    pub(super) fn side_panel(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.device_header(ui);
            ui.separator();
            self.scene_presets(ui);
            ui.separator();
            self.view_section(ui);
            ui.separator();
            self.effects_section(ui);
            self.image_section(ui);
            self.overlays_section(ui);
            self.virtual_camera_section(ui);
            self.audio_section(ui);
        });
    }

    fn device_header(&mut self, ui: &mut Ui) {
        let name = if self.camera_name.is_empty() { "Insta360 Link 2C" } else { &self.camera_name };
        ui.label(egui::RichText::new(name).size(16.0).strong());
        status_dot(ui, self.connected, if self.connected { "Connected" } else { "Searching for camera..." });
        status_dot(
            ui,
            self.config.custom_gestures_enabled,
            if self.config.custom_gestures_enabled { "Custom gestures on" } else { "Custom gestures off" },
        );
    }

    fn scene_presets(&mut self, ui: &mut Ui) {
        section_title(ui, "Scene presets");
        ui.horizontal_wrapped(|ui| {
            for slot in 1..=PRESET_SLOTS {
                let saved = self.config.preset(slot).copied();
                let label = format!("{slot:02}");
                let button = egui::Button::new(egui::RichText::new(label).size(15.0))
                    .min_size(Vec2::new(40.0, 40.0))
                    .stroke(if saved.is_some() { Stroke::new(1.5, theme::ACCENT) } else { Stroke::NONE });
                let response = ui.add(button).on_hover_text(if saved.is_some() {
                    "Click to apply. Right-click for options."
                } else {
                    "Empty. Right-click to save the current view here."
                });
                if response.clicked() {
                    if let Some(preset) = saved {
                        self.apply_preset(preset);
                    }
                }
                response.context_menu(|ui| {
                    if ui.button("Save current view here").clicked() {
                        self.save_preset(slot);
                        ui.close();
                    }
                    if saved.is_some() && ui.button("Delete").clicked() {
                        self.config.presets.retain(|p| p.slot != slot);
                        self.save_config();
                        ui.close();
                    }
                });
            }
        });
        if ui.button("+  Save current view").clicked() {
            let slot = (1..=PRESET_SLOTS).find(|s| self.config.preset(*s).is_none()).unwrap_or(PRESET_SLOTS);
            self.save_preset(slot);
        }
    }

    fn save_preset(&mut self, slot: u8) {
        let Some(state) = self.state else { return };
        let preset = ScenePreset::capture(
            slot,
            state.pan,
            state.tilt,
            state.zoom,
            state.mode.unwrap_or(VideoMode::Normal),
        );
        self.config.upsert_preset(preset);
        self.save_config();
    }

    fn apply_preset(&mut self, preset: ScenePreset) {
        self.service.send(Request::SetZoom(ZoomUnits::from_raw(preset.zoom)));
        self.service.send(Request::SetMode(preset.mode));
    }

    fn view_section(&mut self, ui: &mut Ui) {
        section_title(ui, "View");
        let zoom = self.state.map_or(ZoomUnits::ONE_X, |s| s.zoom);
        let multiplier = zoom.multiplier() as f32;

        // Miniature of the visible region inside the full frame.
        let (rect, _) = ui.allocate_exact_size(VIEW_BOX, Sense::hover());
        ui.painter().rect_filled(rect, 4.0, theme::SURFACE);
        let visible = Rect::from_center_size(rect.center(), VIEW_BOX / multiplier.max(1.0));
        ui.painter().rect_stroke(visible, 2.0, Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
        ui.painter().circle_filled(visible.center(), 4.0, theme::ACCENT);

        ui.horizontal(|ui| {
            let step_units = 25;
            if ui.button("\u{2212}").clicked() {
                self.set_zoom_raw(zoom.raw() as i32 - step_units);
            }
            let mut value = multiplier;
            let slider = egui::Slider::new(&mut value, 1.0..=4.0).step_by(0.05).suffix("x");
            if ui.add(slider).changed() {
                self.set_zoom_raw((value * 100.0).round() as i32);
            }
            if ui.button("+").clicked() {
                self.set_zoom_raw(zoom.raw() as i32 + step_units);
            }
        });
        ui.label(
            egui::RichText::new(
                "Digital pan/tilt is not exposed by the Link 2C firmware; zoom crops the centre.",
            )
            .size(11.0)
            .color(theme::MUTED),
        );
    }

    fn set_zoom_raw(&mut self, raw: i32) {
        let zoom = ZoomUnits::from_raw_clamped(raw as i64, ZOOM_RANGE);
        self.service.send(Request::SetZoom(zoom));
    }

    fn effects_section(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new("Effects").default_open(false).show(ui, |ui| {
            let mut effects = self.config.effects.clone();
            ui.checkbox(&mut effects.blur_background, "Background blur");
            ui.add_enabled(
                effects.blur_background,
                egui::Slider::new(&mut effects.blur_strength, 1..=5).text("Strength"),
            );
            ui.label(
                egui::RichText::new("Applied to the preview, snapshots and recordings.")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            self.commit_effects(effects);
        });
    }

    fn commit_effects(&mut self, effects: linkcam_core::config::EffectsConfig) {
        if effects != self.config.effects {
            self.config.effects = effects.clone();
            self.service.send(Request::SetEffects(effects));
            self.save_config();
        }
    }

    fn image_section(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new("Image").default_open(true).show(ui, |ui| {
            let controls = self.image_controls.clone();
            if controls.is_empty() {
                ui.label(egui::RichText::new("No image controls available").color(theme::MUTED));
            }
            for (control, value) in controls {
                self.image_row(ui, control, value);
            }
        });
    }

    fn image_row(&mut self, ui: &mut Ui, control: ImageControl, value: linkcam_core::ControlValue) {
        let mut current = value.value;
        let mut auto = value.auto;
        ui.horizontal(|ui| {
            ui.label(control.label());
            if matches!(control, ImageControl::WhiteBalance | ImageControl::Exposure)
                && ui.checkbox(&mut auto, "Auto").changed()
            {
                self.service.send(Request::SetImage { control, value: current, auto });
            }
        });
        let slider = egui::Slider::new(&mut current, value.min..=value.max);
        if ui.add_enabled(!auto, slider).changed() {
            self.service.send(Request::SetImage { control, value: current, auto: false });
        }
        ui.add_space(2.0);
    }

    fn audio_section(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new("Audio").default_open(false).show(ui, |ui| {
            let mut audio = self.config.audio;
            ui.checkbox(&mut audio.record_microphone, "Record microphone");
            ui.add_enabled(
                audio.record_microphone,
                egui::Checkbox::new(&mut audio.noise_reduction, "Noise reduction"),
            );
            ui.add_enabled(
                audio.record_microphone && audio.noise_reduction,
                egui::Slider::new(&mut audio.noise_strength, 1.0..=3.0).text("Strength"),
            );
            ui.label(
                egui::RichText::new("Applied to recordings; the live microphone is not rerouted.")
                    .size(11.0)
                    .color(theme::MUTED),
            );
            if audio != self.config.audio {
                self.config.audio = audio;
                self.service.send(Request::SetAudio(audio));
                self.save_config();
            }
        });
    }

    fn virtual_camera_section(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new("Virtual camera").default_open(false).show(ui, |ui| {
            let mut on = self.virtual_camera_on;
            let label = format!("Publish as \"{}\"", linkcam_core::vcam::CAMERA_NAME);
            if ui.checkbox(&mut on, label).changed() {
                self.service.send(Request::VirtualCamera(on));
            }
            let hint = if self.virtual_camera_installed {
                "Other apps (Teams, Zoom, browsers) can pick it; effects and overlays are included."
            } else {
                "First use asks for administrator approval once to register the camera."
            };
            ui.label(egui::RichText::new(hint).size(11.0).color(theme::MUTED));
        });
    }

    fn overlays_section(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new("Overlays").default_open(false).show(ui, |ui| {
            let mut effects = self.config.effects.clone();
            ui.checkbox(&mut effects.overlay_enabled, "Text overlay");
            ui.add_enabled(
                effects.overlay_enabled,
                egui::TextEdit::singleline(&mut effects.overlay_text).hint_text("Text").desired_width(200.0),
            );
            self.commit_effects(effects);
        });
    }
}
