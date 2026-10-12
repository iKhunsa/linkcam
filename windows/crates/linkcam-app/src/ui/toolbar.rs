//! Bottom toolbar: mode buttons in the middle, capture tools on the right.

use super::widgets::{icon_button, round_button};
use super::LinkCamApp;
use crate::service::Request;
use eframe::egui::{self, Align, Layout, Ui};
use linkcam_core::VideoMode;
use std::time::{SystemTime, UNIX_EPOCH};

impl LinkCamApp {
    pub(super) fn toolbar(&mut self, ui: &mut Ui) {
        let active = self.state.and_then(|s| s.mode);
        ui.horizontal_centered(|ui| {
            // Left group: stream controls.
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let preview_icon = if self.previewing { "\u{1F441}" } else { "\u{1F6AB}" };
                if icon_button(ui, preview_icon, "Show / hide preview", self.connected).clicked() {
                    self.toggle_preview();
                }
                self.resolution_picker(ui);
            });

            // Centre group: mode buttons.
            ui.with_layout(Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                ui.horizontal(|ui| {
                    let modes = [
                        ("\u{1F465}", "Auto Framing", VideoMode::AutoFraming),
                        ("\u{25AD}", "Whiteboard", VideoMode::Whiteboard),
                        ("\u{1F5A5}", "DeskView", VideoMode::DeskView),
                        ("\u{2B07}", "Overhead", VideoMode::Overhead),
                    ];
                    for (icon, caption, mode) in modes {
                        let selected = active == Some(mode);
                        if round_button(ui, icon, caption, selected, self.connected).clicked() {
                            self.set_mode(mode);
                        }
                        ui.add_space(8.0);
                    }
                });
            });
        });

        // Right group overlays the same row, anchored to the right edge.
        let row = ui.min_rect();
        let mut right =
            ui.new_child(egui::UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)));
        if icon_button(&mut right, "\u{2699}", "Gesture settings", true).clicked() {
            self.settings_open = !self.settings_open;
        }
        let recording = self.recording_since.is_some();
        let record_icon = if recording { "\u{23F9}" } else { "\u{23FA}" };
        let record_tip = if recording { "Stop recording" } else { "Record video" };
        if icon_button(&mut right, record_icon, record_tip, self.connected && self.previewing).clicked() {
            if recording {
                self.service.send(Request::StopRecording);
            } else {
                self.service.send(Request::StartRecording(media_path("Videos", "recording", "mp4")));
            }
        }
        if let Some(since) = self.recording_since {
            let seconds = since.elapsed().as_secs();
            right.label(
                egui::RichText::new(format!("\u{25CF} {:02}:{:02}", seconds / 60, seconds % 60))
                    .color(super::theme::DANGER),
            );
        }
        if icon_button(&mut right, "\u{1F4F7}", "Snapshot", self.connected && self.previewing).clicked() {
            self.service.send(Request::Snapshot(media_path("Pictures", "snapshot", "png")));
        }
        self.draw_status_line(&mut right);
    }
}

impl LinkCamApp {
    /// Drop-down of the stream formats the camera offers, like the official "4K30" button.
    fn resolution_picker(&mut self, ui: &mut Ui) {
        let current = self.current_format.map_or_else(|| "--".to_string(), |f| f.label());
        let mut chosen = None;
        egui::ComboBox::from_id_salt("resolution")
            .width(96.0)
            .selected_text(current)
            .show_ui(ui, |ui| {
                for format in &self.formats {
                    let text = format!("{}  {}x{}", format.label(), format.width, format.height);
                    let selected = self.current_format == Some(*format);
                    if ui.selectable_label(selected, text).clicked() {
                        chosen = Some(*format);
                    }
                }
            })
            .response
            .on_hover_text("Stream resolution and frame rate");
        if let Some(format) = chosen {
            self.choose_format(format);
        }
    }
}

/// `<user folder>\LinkCam\<prefix>-<unix seconds>.<ext>`, creating the folder on demand.
fn media_path(folder: &str, prefix: &str, extension: &str) -> std::path::PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(folder)
        .join("LinkCam");
    let _ = std::fs::create_dir_all(&base);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    base.join(format!("{prefix}-{stamp}.{extension}"))
}
