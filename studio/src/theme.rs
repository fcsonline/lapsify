//! Studio look: one type scale, a high-contrast dark palette, and an amber
//! accent shared with the keyframe markers.

use egui::{Color32, CornerRadius, FontFamily, FontId, TextStyle};

pub const ACCENT: Color32 = Color32::from_rgb(255, 196, 70);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(150, 118, 48);
pub const PREVIEW_BG: Color32 = Color32::from_rgb(8, 8, 10);

pub const TEXT: Color32 = Color32::from_rgb(232, 232, 236);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(158, 158, 168);

const PANEL: Color32 = Color32::from_rgb(24, 24, 29);
const SUNKEN: Color32 = Color32::from_rgb(13, 13, 16);
const WIDGET: Color32 = Color32::from_rgb(44, 44, 53);
const WIDGET_HOVER: Color32 = Color32::from_rgb(58, 58, 70);
const WIDGET_ACTIVE: Color32 = Color32::from_rgb(70, 66, 52);
const OUTLINE: Color32 = Color32::from_rgb(58, 58, 66);

/// Use the system typeface when running on macOS: San Francisco for UI text
/// and SF Mono for monospace. Loaded from the OS at runtime (never bundled);
/// silently keeps egui's defaults if anything fails to parse.
#[cfg(target_os = "macos")]
fn install_system_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut changed = false;

    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        fonts.font_data.insert(
            "sf-pro".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "sf-pro".to_owned());
        changed = true;
    }
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNSMono.ttf") {
        fonts.font_data.insert(
            "sf-mono".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "sf-mono".to_owned());
        changed = true;
    }

    if changed {
        // egui panics on unparsable fonts; a corrupt system font must not
        // take the app down.
        let probe = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts.clone());
            let _ = ctx.run(Default::default(), |_| {});
        }));
        if probe.is_ok() {
            ctx.set_fonts(fonts);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn install_system_fonts(_ctx: &egui::Context) {}

pub fn apply(ctx: &egui::Context) {
    install_system_fonts(ctx);

    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(16.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 22.0;

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = SUNKEN;
    visuals.faint_bg_color = Color32::from_rgb(32, 32, 38);

    visuals.override_text_color = Some(TEXT);

    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, OUTLINE);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_WEAK);

    visuals.widgets.inactive.bg_fill = WIDGET;
    visuals.widgets.inactive.weak_bg_fill = WIDGET;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);

    visuals.widgets.hovered.bg_fill = WIDGET_HOVER;
    visuals.widgets.hovered.weak_bg_fill = WIDGET_HOVER;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT_DIM);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, TEXT);

    visuals.widgets.active.bg_fill = WIDGET_ACTIVE;
    visuals.widgets.active.weak_bg_fill = WIDGET_ACTIVE;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.5, TEXT);

    visuals.widgets.open.bg_fill = WIDGET_ACTIVE;
    visuals.widgets.open.weak_bg_fill = WIDGET_ACTIVE;

    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.slider_trailing_fill = true;

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(5);
    }

    style.visuals = visuals;
    ctx.set_style(style);
}
