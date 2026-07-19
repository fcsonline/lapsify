//! Bottom panel: frame scrubber plus the layer curve graph with editable
//! keyframes for the selected parameter.

use egui_plot::{Line, Plot, PlotPoint, PlotPoints, Points, VLine};

const SOURCE_COLOR: egui::Color32 = egui::Color32::from_rgb(90, 160, 255);
const DEVELOPED_COLOR: egui::Color32 = egui::Color32::from_rgb(240, 120, 200);
const COMPENSATION_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 165, 60);
const DEFLICKER_COLOR: egui::Color32 = egui::Color32::from_rgb(90, 200, 120);
use lapsify::Curve;

use crate::app::StudioApp;
use crate::document::ParamId;

/// Luminance series are plotted in EV relative to their own mean, so they
/// share an axis with the exposure layers.
fn relative_ev(values: &[f32]) -> Vec<[f64; 2]> {
    let mean =
        (values.iter().map(|v| *v as f64).sum::<f64>() / values.len().max(1) as f64).max(1e-6);
    values
        .iter()
        .enumerate()
        .map(|(i, v)| [i as f64, ((*v as f64).max(1e-6) / mean).log2()])
        .collect()
}

pub fn show(app: &mut StudioApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("timeline")
        .resizable(true)
        .default_height(240.0)
        .show(ctx, |ui| {
            let Some(doc) = &mut app.doc else {
                ui.disable();
                return;
            };
            let frame_count = doc.frame_count();
            if frame_count == 0 {
                return;
            }
            let last_frame = (frame_count - 1) as u32;

            // ---- scrubber row -----------------------------------------
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() * 0.45).max(220.0);
                let mut frame = app.current_frame;
                let slider = ui.add(
                    egui::Slider::new(&mut frame, 0..=last_frame)
                        .text("frame")
                        .clamping(egui::SliderClamping::Always),
                );
                if slider.changed() {
                    app.current_frame = frame;
                    app.preview_dirty = true;
                }

                ui.separator();
                ui.label("Edit curve:");
                egui::ComboBox::from_id_salt("edit_param")
                    .selected_text(app.selected_param.label())
                    .show_ui(ui, |ui| {
                        for param in ParamId::ALL {
                            if ui
                                .selectable_label(app.selected_param == param, param.label())
                                .clicked()
                            {
                                app.selected_param = param;
                                app.dragging_keyframe = None;
                            }
                        }
                    });

                // Layer visibility chips, color-coded to the plot lines and
                // shown only for layers that exist. They double as a legend.
                let analysis = doc.project.analysis.as_ref();
                let mut chips: Vec<(&str, egui::Color32, &mut bool, bool)> = vec![
                    (
                        "Source",
                        SOURCE_COLOR,
                        &mut app.show_source_luma,
                        analysis.is_some_and(|a| a.source_luminance.is_some()),
                    ),
                    (
                        "Developed",
                        DEVELOPED_COLOR,
                        &mut app.show_developed_luma,
                        analysis.is_some_and(|a| a.developed_luminance.is_some()),
                    ),
                    (
                        "Compensation",
                        COMPENSATION_COLOR,
                        &mut app.show_compensation,
                        analysis.is_some_and(|a| a.holy_grail.is_some()),
                    ),
                    (
                        "Deflicker",
                        DEFLICKER_COLOR,
                        &mut app.show_deflicker,
                        analysis.is_some_and(|a| a.deflicker.is_some()),
                    ),
                ];
                if chips.iter().any(|(_, _, _, exists)| *exists) {
                    ui.separator();
                    ui.label(egui::RichText::new("Layers:").color(crate::theme::TEXT_WEAK));
                    for (name, color, on, exists) in chips.iter_mut() {
                        if !*exists {
                            continue;
                        }
                        let text = egui::RichText::new(*name).color(if **on {
                            *color
                        } else {
                            crate::theme::TEXT_WEAK
                        });
                        if ui.selectable_label(**on, text).clicked() {
                            **on = !**on;
                        }
                    }
                }
            });

            // ---- curve plot -------------------------------------------
            let param = app.selected_param;
            let (min_v, max_v) = param.range();
            let analysis = doc.project.analysis.clone();
            let param_curve = doc.curve(param).clone();

            let mut edited: Option<EditAction> = None;

            let plot = Plot::new("layer_curves")
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_boxed_zoom(false)
                .include_x(0.0)
                .include_x(last_frame as f64)
                .show_x(true)
                .show_y(true);

            plot.show(ui, |plot_ui| {
                // Background layers: thin and dimmed so the edited curve
                // stays the focus. Each is opt-in via its chip.
                let faint = |c: egui::Color32| c.gamma_multiply(0.55);
                if let Some(analysis) = &analysis {
                    if app.show_source_luma {
                        if let Some(series) = &analysis.source_luminance {
                            plot_ui.line(
                                Line::new(
                                    "source luminance",
                                    PlotPoints::from(relative_ev(&series.values)),
                                )
                                .color(faint(SOURCE_COLOR))
                                .width(1.0),
                            );
                        }
                    }
                    if app.show_developed_luma {
                        if let Some(series) = &analysis.developed_luminance {
                            plot_ui.line(
                                Line::new(
                                    "developed luminance",
                                    PlotPoints::from(relative_ev(&series.values)),
                                )
                                .color(faint(DEVELOPED_COLOR))
                                .width(1.0),
                            );
                        }
                    }
                    if app.show_compensation {
                        if let Some(hg) = &analysis.holy_grail {
                            let points: Vec<[f64; 2]> = (0..frame_count)
                                .map(|i| [i as f64, hg.effective(i) as f64])
                                .collect();
                            plot_ui.line(
                                Line::new("EXIF compensation", PlotPoints::from(points))
                                    .color(faint(COMPENSATION_COLOR))
                                    .width(1.0),
                            );
                        }
                    }
                    if app.show_deflicker {
                        if let Some(deflicker) = &analysis.deflicker {
                            let points: Vec<[f64; 2]> = (0..frame_count)
                                .map(|i| [i as f64, deflicker.offset(i) as f64])
                                .collect();
                            plot_ui.line(
                                Line::new("deflicker", PlotPoints::from(points))
                                    .color(faint(DEFLICKER_COLOR))
                                    .width(1.0),
                            );
                        }
                    }
                }

                // The selected parameter's curve, sampled per frame.
                let sampled: Vec<[f64; 2]> = (0..=last_frame)
                    .map(|f| [f as f64, param_curve.sample(f) as f64])
                    .collect();
                plot_ui.line(
                    Line::new(param.label(), PlotPoints::from(sampled))
                        .color(crate::theme::ACCENT)
                        .width(2.5),
                );

                // Playhead.
                plot_ui.vline(
                    VLine::new("playhead", app.current_frame as f64)
                        .color(egui::Color32::from_gray(200))
                        .width(1.5),
                );

                // ---- keyframe editing ---------------------------------
                let keyframes: Vec<(u32, f32)> = match &param_curve {
                    Curve::Keyframed(kfs) => kfs.iter().map(|k| (k.frame, k.value)).collect(),
                    Curve::Constant(_) => Vec::new(),
                };

                if !keyframes.is_empty() {
                    let marker_points: Vec<[f64; 2]> = keyframes
                        .iter()
                        .map(|(f, v)| [*f as f64, *v as f64])
                        .collect();
                    plot_ui.points(
                        Points::new("keyframes", PlotPoints::from(marker_points))
                            .radius(5.5)
                            .color(crate::theme::ACCENT),
                    );
                }

                let response = plot_ui.response().clone();
                let pointer = plot_ui.pointer_coordinate();

                // Nearest keyframe to the pointer, in screen space.
                let nearest = pointer.and_then(|p| {
                    let cursor = plot_ui.screen_from_plot(p);
                    keyframes
                        .iter()
                        .enumerate()
                        .map(|(i, (f, v))| {
                            let screen =
                                plot_ui.screen_from_plot(PlotPoint::new(*f as f64, *v as f64));
                            (i, cursor.distance(screen))
                        })
                        .filter(|(_, d)| *d < 12.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(i, _)| i)
                });

                if response.drag_started() {
                    app.dragging_keyframe = nearest;
                }
                if response.dragged() {
                    if let (Some(index), Some(p)) = (app.dragging_keyframe, pointer) {
                        edited = Some(EditAction::Drag {
                            index,
                            frame: (p.x.round() as i64).clamp(0, last_frame as i64) as u32,
                            value: (p.y as f32).clamp(min_v, max_v),
                        });
                    }
                }
                if response.drag_stopped() {
                    app.dragging_keyframe = None;
                }
                if response.double_clicked() {
                    if let Some(p) = pointer {
                        edited = Some(EditAction::Add {
                            frame: (p.x.round() as i64).clamp(0, last_frame as i64) as u32,
                            value: (p.y as f32).clamp(min_v, max_v),
                        });
                    }
                } else if response.secondary_clicked() {
                    if let Some(index) = nearest {
                        edited = Some(EditAction::Delete { index });
                    }
                } else if response.clicked() && nearest.is_none() {
                    // Plain click scrubs the playhead.
                    if let Some(p) = pointer {
                        app.current_frame = (p.x.round() as i64).clamp(0, last_frame as i64) as u32;
                        app.preview_dirty = true;
                    }
                }
            });

            if let Some(action) = edited {
                apply_edit(doc, param, action);
                app.preview_dirty = true;
            }
        });
}

enum EditAction {
    Drag {
        index: usize,
        frame: u32,
        value: f32,
    },
    Add {
        frame: u32,
        value: f32,
    },
    Delete {
        index: usize,
    },
}

fn apply_edit(doc: &mut crate::document::Document, param: ParamId, action: EditAction) {
    let curve = doc.curve_mut(param);
    match action {
        EditAction::Add { frame, value } => match curve {
            Curve::Constant(_) => {
                *curve = Curve::Keyframed(vec![lapsify::Keyframe::new(frame, value)]);
            }
            Curve::Keyframed(kfs) => match kfs.binary_search_by_key(&frame, |k| k.frame) {
                Ok(i) => kfs[i].value = value,
                Err(i) => kfs.insert(i, lapsify::Keyframe::new(frame, value)),
            },
        },
        EditAction::Delete { index } => {
            if let Curve::Keyframed(kfs) = curve {
                if index < kfs.len() {
                    kfs.remove(index);
                    if kfs.is_empty() {
                        *curve = Curve::Constant(param.neutral());
                    }
                }
            }
        }
        EditAction::Drag {
            index,
            frame,
            value,
        } => {
            if let Curve::Keyframed(kfs) = curve {
                if index < kfs.len() {
                    // Keep the keyframe strictly between its neighbors.
                    let lo = if index > 0 {
                        kfs[index - 1].frame + 1
                    } else {
                        0
                    };
                    let hi = kfs
                        .get(index + 1)
                        .map(|k| k.frame.saturating_sub(1))
                        .unwrap_or(u32::MAX);
                    kfs[index].frame = frame.clamp(lo, hi.max(lo));
                    kfs[index].value = value;
                }
            }
        }
    }
    doc.dirty = true;
}
