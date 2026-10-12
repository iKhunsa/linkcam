//! Dark palette modelled on the official Insta360 Link Controller.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const BACKGROUND: Color32 = Color32::from_rgb(0x1a, 0x1e, 0x22);
pub const PANEL: Color32 = Color32::from_rgb(0x22, 0x26, 0x2b);
pub const SURFACE: Color32 = Color32::from_rgb(0x2c, 0x31, 0x37);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x36, 0x3c, 0x43);
pub const ACCENT: Color32 = Color32::from_rgb(0x1f, 0xd6, 0x86);
pub const TEXT: Color32 = Color32::from_rgb(0xdc, 0xdf, 0xe2);
pub const MUTED: Color32 = Color32::from_rgb(0x8c, 0x93, 0x9a);
pub const DANGER: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

const WIDGET_RADIUS: u8 = 6;

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = BACKGROUND;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;

    let radius = CornerRadius::same(WIDGET_RADIUS);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
    }
    visuals.widgets.inactive.bg_fill = SURFACE;
    visuals.widgets.inactive.weak_bg_fill = SURFACE;
    visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
    visuals.widgets.hovered.weak_bg_fill = SURFACE_HOVER;
    visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.6);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, SURFACE);
    ctx.set_visuals(visuals);
}
