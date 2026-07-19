//! The guided workflow: the steps to process a time-lapse, in order, with
//! per-step status. This is where analysis actions live in the UI.

use crate::app::StudioApp;
use crate::document::Document;
use lapsify::Curve;

/// Action picked in the workflow UI, applied by the caller once document
/// borrows are released.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkflowAction {
    Luminance,
    Compensate,
    SuggestKeyframes,
    Deflicker,
    Export,
}

pub fn apply(app: &mut StudioApp, action: WorkflowAction) {
    match action {
        WorkflowAction::Luminance => app.job_luminance(false),
        WorkflowAction::Compensate => app.job_holygrail(),
        WorkflowAction::SuggestKeyframes => app.job_suggest_keyframes(),
        WorkflowAction::Deflicker => app.job_deflicker(),
        WorkflowAction::Export => app.job_render(),
    }
}

/// Draw the workflow section. Returns the action to run, if any.
pub fn section(ui: &mut egui::Ui, doc: &Document, idle: bool) -> Option<WorkflowAction> {
    let mut action = None;

    let analysis = doc.project.analysis.as_ref();
    let has_luma = analysis.is_some_and(|a| a.source_luminance.is_some());
    let has_compensation = analysis.is_some_and(|a| a.holy_grail.is_some());
    let has_deflicker = analysis.is_some_and(|a| a.deflicker.is_some());
    let keyframes = match &doc.project.color.exposure {
        Curve::Keyframed(kfs) => kfs.len(),
        Curve::Constant(_) => 0,
    };

    ui.add_space(6.0);
    ui.heading("Workflow");
    ui.add_space(4.0);

    let mut step = |ui: &mut egui::Ui,
                    number: &str,
                    done: bool,
                    title: &str,
                    detail: &str,
                    button: Option<(&str, WorkflowAction)>| {
        ui.horizontal(|ui| {
            let badge = if done { "✔" } else { number };
            let badge_color = if done {
                egui::Color32::from_rgb(90, 200, 120)
            } else {
                crate::theme::TEXT_WEAK
            };
            ui.allocate_ui_with_layout(
                egui::vec2(16.0, 18.0),
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    ui.label(egui::RichText::new(badge).color(badge_color).strong());
                },
            );
            ui.label(egui::RichText::new(title).strong());
            if let Some((label, act)) = button {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(idle, egui::Button::new(label).small())
                        .clicked()
                    {
                        action = Some(act);
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(detail)
                        .color(crate::theme::TEXT_WEAK)
                        .size(11.0),
                )
                .wrap(),
            );
        });
        ui.add_space(4.0);
    };

    step(
        ui,
        "1",
        has_luma,
        "Measure brightness",
        "Scan every frame's brightness — feeds the graph and later steps.",
        Some(("Run", WorkflowAction::Luminance)),
    );
    step(
        ui,
        "2",
        has_compensation,
        "Fix camera jumps",
        "Cancel shutter/ISO exposure steps (day-to-night shots, uses EXIF).",
        Some(("Run", WorkflowAction::Compensate)),
    );
    step(
        ui,
        "3",
        keyframes > 0,
        "Create your look",
        if keyframes > 0 {
            "Drag keyframes in the timeline, shape it with the sliders below."
        } else {
            "Place keyframes where brightness moves, then shape the look below."
        },
        Some(("Suggest", WorkflowAction::SuggestKeyframes)),
    );
    step(
        ui,
        "4",
        has_deflicker,
        "Deflicker",
        "Smooth leftover flicker. Run once your look is set.",
        Some(("Run", WorkflowAction::Deflicker)),
    );
    step(
        ui,
        "5",
        false,
        "Export",
        "Render with the export settings below.",
        Some(("Export", WorkflowAction::Export)),
    );

    ui.add_space(4.0);
    ui.separator();

    action
}
