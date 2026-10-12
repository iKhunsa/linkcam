//! Small custom widgets shared by the panels.

use super::theme;
use eframe::egui::{self, Align2, FontId, Pos2, Response, Sense, Stroke, Ui, Vec2};

const ROUND_BUTTON_RADIUS: f32 = 26.0;
const ROUND_BUTTON_LABEL_GAP: f32 = 6.0;

/// Circular toolbar button with a caption underneath, like the official
/// controller's Auto Framing / Whiteboard / DeskView buttons.
pub fn round_button(ui: &mut Ui, icon: &str, caption: &str, selected: bool, enabled: bool) -> Response {
    let size = Vec2::new(ROUND_BUTTON_RADIUS * 2.0 + 28.0, ROUND_BUTTON_RADIUS * 2.0 + 28.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let painter = ui.painter();
    let center = Pos2::new(rect.center().x, rect.top() + ROUND_BUTTON_RADIUS);

    let fill = if selected {
        theme::ACCENT.gamma_multiply(0.25)
    } else if response.hovered() && enabled {
        theme::SURFACE_HOVER
    } else {
        theme::SURFACE
    };
    let ring = if selected { Stroke::new(2.0, theme::ACCENT) } else { Stroke::NONE };
    painter.circle(center, ROUND_BUTTON_RADIUS, fill, ring);

    let ink = if !enabled {
        theme::MUTED.gamma_multiply(0.5)
    } else if selected {
        theme::ACCENT
    } else {
        theme::TEXT
    };
    painter.text(center, Align2::CENTER_CENTER, icon, FontId::proportional(22.0), ink);
    painter.text(
        Pos2::new(rect.center().x, center.y + ROUND_BUTTON_RADIUS + ROUND_BUTTON_LABEL_GAP),
        Align2::CENTER_TOP,
        caption,
        FontId::proportional(12.5),
        if enabled { theme::MUTED } else { theme::MUTED.gamma_multiply(0.5) },
    );
    response
}

/// Small flat icon button used for the right-hand toolbar group.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str, enabled: bool) -> Response {
    let button = egui::Button::new(egui::RichText::new(icon).size(18.0))
        .min_size(Vec2::splat(38.0))
        .fill(theme::SURFACE);
    ui.add_enabled(enabled, button).on_hover_text(tooltip)
}

/// Section title with a thin rule, e.g. "Scene presets".
pub fn section_title(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).strong().size(14.0));
    ui.add_space(4.0);
}

/// A coloured dot plus text, used for connection and gesture status.
pub fn status_dot(ui: &mut Ui, on: bool, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        let color = if on { theme::ACCENT } else { theme::MUTED };
        ui.painter().circle_filled(rect.center(), 4.0, color);
        ui.label(egui::RichText::new(text).color(theme::MUTED).size(12.0));
    });
}
