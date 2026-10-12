//! Gesture settings window: engine switch, live detection readout and the
//! gesture -> action binding editor.

use super::{theme, LinkCamApp};
use crate::service::Request;
use eframe::egui::{self, ComboBox, Context, Ui};
use linkcam_core::gesture::{Action, Binding, CameraCommand, Gesture};
use linkcam_core::VideoMode;

const ACTION_KEYS: &str = "Press keys";
const ACTION_LAUNCH: &str = "Launch";
const ACTION_MODE: &str = "Camera mode";
const ACTION_ZOOM: &str = "Zoom";
const ACTION_RECENTER: &str = "Recenter";

fn action_kind(action: &Action) -> &'static str {
    match action {
        Action::Keys { .. } => ACTION_KEYS,
        Action::Launch { .. } => ACTION_LAUNCH,
        Action::Camera(CameraCommand::SetMode { .. } | CameraCommand::ToggleMode { .. }) => ACTION_MODE,
        Action::Camera(CameraCommand::ZoomBy { .. }) => ACTION_ZOOM,
        Action::Camera(CameraCommand::Recenter) => ACTION_RECENTER,
    }
}

fn default_action(kind: &str) -> Action {
    match kind {
        ACTION_LAUNCH => Action::Launch { target: "notepad.exe".into() },
        ACTION_MODE => Action::Camera(CameraCommand::ToggleMode { mode: VideoMode::DeskView }),
        ACTION_ZOOM => Action::Camera(CameraCommand::ZoomBy { percent: 50 }),
        ACTION_RECENTER => Action::Camera(CameraCommand::Recenter),
        _ => Action::Keys { keys: vec!["ctrl".into(), "alt".into(), "t".into()] },
    }
}

impl LinkCamApp {
    pub(super) fn gestures_window(&mut self, ctx: &Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Gesture control")
            .open(&mut open)
            .default_width(560.0)
            .collapsible(false)
            .show(ctx, |ui| self.gestures_contents(ui));
        self.settings_open = open;
    }

    fn gestures_contents(&mut self, ui: &mut Ui) {
        ui.label(
            egui::RichText::new(
                "The camera's own palm / V / L gestures always run in its firmware. \
                 Custom gestures are detected on this PC from the video stream.",
            )
            .size(12.0)
            .color(theme::MUTED),
        );
        ui.add_space(6.0);

        let mut enabled = self.config.custom_gestures_enabled;
        if ui.checkbox(&mut enabled, "Enable custom gestures (analyses frames on this PC)").changed() {
            self.config.custom_gestures_enabled = enabled;
            self.service.send(Request::GesturesEnabled(enabled));
            if enabled {
                self.service.send(Request::StartPreview);
                self.previewing = true;
            }
            self.save_config();
        }
        let seeing = self.pose.map_or("no hand", Gesture::label);
        ui.label(egui::RichText::new(format!("Seeing: {seeing}")).color(theme::ACCENT));
        ui.separator();

        let mut changed = false;
        let mut remove = None;
        for (index, binding) in self.config.bindings.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut binding.enabled, "").changed();
                changed |= gesture_combo(ui, index, &mut binding.gesture);
                ui.label("\u{2192}");
                changed |= action_editor(ui, index, &mut binding.action);
                if !binding.gesture.is_motion() {
                    changed |= ui
                        .add(egui::DragValue::new(&mut binding.hold_ms).range(0..=3000).suffix(" ms hold"))
                        .changed();
                }
                if ui.button("\u{1F5D1}").on_hover_text("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.config.bindings.remove(index);
            changed = true;
        }
        if ui.button("+ Add binding").clicked() {
            self.config.bindings.push(Binding::new(Gesture::Peace, default_action(ACTION_KEYS)));
            changed = true;
        }
        if changed {
            self.service.send(Request::Bindings(self.config.bindings.clone()));
            self.save_config();
        }
    }
}

fn gesture_combo(ui: &mut Ui, index: usize, gesture: &mut Gesture) -> bool {
    let before = *gesture;
    ComboBox::from_id_salt(("gesture", index)).width(110.0).selected_text(gesture.label()).show_ui(
        ui,
        |ui| {
            for candidate in Gesture::ALL {
                ui.selectable_value(gesture, candidate, candidate.label());
            }
        },
    );
    *gesture != before
}

fn action_editor(ui: &mut Ui, index: usize, action: &mut Action) -> bool {
    let mut changed = false;
    let mut kind = action_kind(action);
    ComboBox::from_id_salt(("action", index)).width(110.0).selected_text(kind).show_ui(ui, |ui| {
        for candidate in [ACTION_KEYS, ACTION_LAUNCH, ACTION_MODE, ACTION_ZOOM, ACTION_RECENTER] {
            ui.selectable_value(&mut kind, candidate, candidate);
        }
    });
    if kind != action_kind(action) {
        *action = default_action(kind);
        changed = true;
    }
    match action {
        Action::Keys { keys } => {
            let mut text = keys.join("+");
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(150.0).hint_text("ctrl+alt+t"))
                .changed()
            {
                *keys = text.split('+').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
                changed = true;
            }
        }
        Action::Launch { target } => {
            changed |= ui.add(egui::TextEdit::singleline(target).desired_width(150.0)).changed();
        }
        Action::Camera(CameraCommand::SetMode { mode } | CameraCommand::ToggleMode { mode }) => {
            let before = *mode;
            ComboBox::from_id_salt(("mode", index)).selected_text(mode.label()).show_ui(ui, |ui| {
                for candidate in VideoMode::ALL {
                    ui.selectable_value(mode, candidate, candidate.label());
                }
            });
            changed |= *mode != before;
        }
        Action::Camera(CameraCommand::ZoomBy { percent }) => {
            changed |= ui.add(egui::DragValue::new(percent).range(-300..=300).suffix(" %")).changed();
        }
        Action::Camera(CameraCommand::Recenter) => {}
    }
    changed
}
