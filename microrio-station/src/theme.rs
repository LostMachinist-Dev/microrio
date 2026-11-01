//! Instrument styling, carried over from the 2026 GUI.
//!
//! The palette is lifted from `archive/giga-gui/public/well developed gui.txt`
//! and the trace colours from the original six-channel scope sketch.
//!
//! The original used Bahnschrift / DIN Alternate. Those are Windows fonts and
//! egui wants font bytes rather than a system lookup, so the instrument feel
//! here comes from monospaced numerics and spaced uppercase headers instead.

use egui::{Color32, FontFamily, FontId, RichText, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
pub const PANEL: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
pub const HEADER_BG: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
pub const BORDER: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub const BORDER_BRIGHT: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
pub const TEXT: Color32 = Color32::from_rgb(0xEE, 0xEE, 0xEE);
pub const MUTED: Color32 = Color32::from_rgb(0x88, 0x88, 0x88);
pub const HEADER_TEXT: Color32 = Color32::from_rgb(0xCC, 0xCC, 0xCC);

pub const LED_OFF: Color32 = Color32::from_rgb(0x30, 0x00, 0x00);
pub const LED_OFF_EDGE: Color32 = Color32::from_rgb(0x60, 0x00, 0x00);
pub const LED_ON: Color32 = Color32::from_rgb(218, 48, 5);

/// "Fine" is ordinary text: colour is kept for things that need attention.
pub const OK: Color32 = TEXT;
pub const WARN: Color32 = Color32::from_rgb(0xFF, 0xAA, 0x33);
pub const BAD: Color32 = Color32::from_rgb(0xFF, 0x55, 0x55);

/// Trace colours from the original scope: `6Achannels.ino`.
pub const TRACES: [Color32; 6] = [
    Color32::from_rgb(0xFF, 0x55, 0x55),
    Color32::from_rgb(0x55, 0xFF, 0x55),
    Color32::from_rgb(0x55, 0x99, 0xFF),
    Color32::from_rgb(0xFF, 0xAA, 0x33),
    Color32::from_rgb(0xCC, 0x66, 0xFF),
    Color32::from_rgb(0x66, 0xFF, 0xFF),
];

pub fn trace(i: usize) -> Color32 {
    TRACES[i % TRACES.len()]
}

pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = Color32::from_rgb(0x0C, 0x0C, 0x0C);
    visuals.faint_bg_color = Color32::from_rgb(0x1C, 0x1C, 0x1C);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.bg_fill = HEADER_BG;
    visuals.widgets.inactive.fg_stroke.color = TEXT;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER_BRIGHT);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, BORDER_BRIGHT);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x33, 0x33, 0x33);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0x3A, 0x3A, 0x3A);
    visuals.selection.bg_fill = Color32::from_rgb(0x2A, 0x4A, 0x6A);
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(16.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
            (
                TextStyle::Button,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    });
}

/// A column heading: small, spaced, uppercase — the 2026 look.
pub fn header(text: &str) -> RichText {
    let spaced: String = text
        .to_uppercase()
        .chars()
        .flat_map(|c| [c, ' '])
        .collect::<String>()
        .trim_end()
        .to_string();
    RichText::new(spaced).size(10.0).color(HEADER_TEXT).strong()
}

/// A numeric readout. Monospaced so digits do not jitter as values change.
pub fn value(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).monospace().color(TEXT)
}

pub fn value_muted(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).monospace().color(MUTED)
}

/// A clickable indicator lamp, in the style of the original control grid.
pub fn lamp(ui: &mut egui::Ui, on: bool, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let (fill, edge) = if on {
        (LED_ON, Color32::from_rgb(0x8F, 0xFF, 0x8F))
    } else {
        (LED_OFF, LED_OFF_EDGE)
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(3), fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Inside,
    );
    resp
}
