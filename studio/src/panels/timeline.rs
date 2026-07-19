//! Bottom panel: frame scrubber plus the layer curve graph with editable
//! keyframes for the selected parameter.

use egui_plot::{Legend, Line, Plot, PlotPoint, PlotPoints, Points, VLine};
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
                ui.label(
                    egui::RichText::new(
                        "double-click: add keyframe · drag: move · right-click: delete",
                    )
                    .weak()
                    .size(11.0),
                );
            });

            // ---- curve plot -------------------------------------------
            let param = app.selected_param;
            let (min_v, max_v) = param.range();
            let analysis = doc.project.analysis.clone();
            let param_curve = doc.curve(param).clone();

            let mut edited: Option<EditAction> = None;

            let plot = Plot::new("layer_curves")
                .legend(Legend::default().position(egui_plot::Corner::LeftTop))
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_boxed_zoom(false)
                .include_x(0.0)
                .include_x(last_frame as f64)
                .show_x(true)
                .show_y(true);

            plot.show(ui, |plot_ui| {
                // Measured luminance layers (relative EV).
                if let Some(analysis) = &analysis {
                    if let Some(series) = &analysis.source_luminance {
                        plot_ui.line(
                            Line::new(
                                "source luminance",
                                PlotPoints::from(relative_ev(&series.values)),
                            )
                            .color(egui::Color32::from_rgb(90, 160, 255)),
                        );
                    }
                    if let Some(series) = &analysis.developed_luminance {
                        plot_ui.line(
                            Line::new(
                                "developed luminance",
                                PlotPoints::from(relative_ev(&series.values)),
                            )
                            .color(egui::Color32::from_rgb(240, 120, 200)),
                        );
                    }
                    if let Some(hg) = &analysis.holy_grail {
                        let points: Vec<[f64; 2]> = (0..frame_count)
                            .map(|i| [i as f64, hg.effective(i) as f64])
                            .collect();
                        plot_ui.line(
                            Line::new("EXIF compensation", PlotPoints::from(points))
                                .color(egui::Color32::from_rgb(255, 165, 60)),
                        );
                    }
                    if let Some(deflicker) = &analysis.deflicker {
                        let points: Vec<[f64; 2]> = (0..frame_count)
                            .map(|i| [i as f64, deflicker.offset(i) as f64])
                            .collect();
                        plot_ui.line(
                            Line::new("deflicker", PlotPoints::from(points))
                                .color(egui::Color32::from_rgb(90, 200, 120)),
                        );
                    }
                }

                // The selected parameter's curve, sampled per frame.
                let sampled: Vec<[f64; 2]> = (0..=last_frame)
                    .map(|f| [f as f64, param_curve.sample(f) as f64])
                    .collect();
                plot_ui.line(
                    Line::new(param.label(), PlotPoints::from(sampled))
                        .color(egui::Color32::from_rgb(250, 220, 90))
                        .width(2.0),
                );

                // Playhead.
                plot_ui.vline(
                    VLine::new("playhead", app.current_frame as f64)
                        .color(egui::Color32::from_gray(160)),
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
                            .radius(5.0)
                            .color(egui::Color32::from_rgb(255, 240, 150)),
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
