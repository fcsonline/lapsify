use lapsify::project::Codec;
use lapsify::Curve;

use crate::app::StudioApp;
use crate::document::ParamId;

pub fn show(app: &mut StudioApp, ctx: &egui::Context) {
    egui::SidePanel::right("adjustments")
        .default_width(280.0)
        .show(ctx, |ui| {
            let Some(doc) = &mut app.doc else {
                ui.disable();
                return;
            };
            let frame = app.current_frame;
            let mut changed = false;

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(6.0);
                ui.heading("Adjustments");
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(
                        "Yellow dot = keyframed: the slider edits the keyframe at the \
                         current frame. Click the dot to add or remove a keyframe.",
                    )
                    .weak()
                    .size(11.0),
                );
                ui.add_space(6.0);

                for param in ParamId::ALL {
                    let (min, max) = param.range();
                    let keyframed = matches!(doc.curve(param), Curve::Keyframed(_));
                    let on_key = doc.has_keyframe_at(param, frame);
                    let mut value = doc.value_at(param, frame);

                    ui.horizontal(|ui| {
                        // Keyframe toggle dot: filled when a keyframe sits on
                        // the current frame, tinted while the curve is
                        // keyframed at all.
                        let color = if on_key {
                            egui::Color32::from_rgb(250, 220, 90)
                        } else if keyframed {
                            egui::Color32::from_rgb(150, 135, 70)
                        } else {
                            ui.visuals().weak_text_color()
                        };
                        let symbol = if on_key { "⏺" } else { "○" };
                        let dot = ui
                            .add(
                                egui::Button::new(egui::RichText::new(symbol).color(color))
                                    .frame(false),
                            )
                            .on_hover_text(if keyframed {
                                "Keyframed curve — click to add/remove a keyframe here"
                            } else {
                                "Constant — click to start keyframing this parameter"
                            });
                        if dot.clicked() {
                            doc.toggle_keyframe(param, frame);
                            changed = true;
                        }

                        let slider = ui.add(
                            egui::Slider::new(&mut value, min..=max)
                                .text(param.label())
                                .clamping(egui::SliderClamping::Always),
                        );
                        if slider.changed() {
                            doc.set_value(param, frame, value);
                            changed = true;
                        }
                        if slider.double_clicked() {
                            doc.set_value(param, frame, param.neutral());
                            changed = true;
                        }
                    });
                }

                ui.add_space(10.0);
                ui.separator();
                ui.heading("Export");
                ui.add_space(4.0);

                let export = &mut doc.project.export;
                egui::Grid::new("export_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Format");
                        egui::ComboBox::from_id_salt("format")
                            .selected_text(&export.format)
                            .show_ui(ui, |ui| {
                                for format in ["mp4", "mov", "jpg", "png", "tiff"] {
                                    if ui
                                        .selectable_label(export.format == format, format)
                                        .clicked()
                                    {
                                        export.format = format.to_string();
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label("Codec");
                        egui::ComboBox::from_id_salt("codec")
                            .selected_text(format!("{:?}", export.codec))
                            .show_ui(ui, |ui| {
                                for codec in [Codec::H264, Codec::H265, Codec::Prores] {
                                    if ui
                                        .selectable_label(
                                            export.codec == codec,
                                            format!("{codec:?}"),
                                        )
                                        .clicked()
                                    {
                                        export.codec = codec;
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();

                        ui.label("FPS");
                        let mut fps = export.fps;
                        if ui
                            .add(egui::DragValue::new(&mut fps).range(1..=120))
                            .changed()
                        {
                            export.fps = fps;
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Quality (CRF)");
                        let mut quality = export.quality;
                        if ui
                            .add(egui::DragValue::new(&mut quality).range(0..=51))
                            .changed()
                        {
                            export.quality = quality;
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Motion blur");
                        let mut blur = export.motion_blur.unwrap_or(1);
                        if ui
                            .add(
                                egui::DragValue::new(&mut blur)
                                    .range(1..=128)
                                    .suffix(" frames"),
                            )
                            .changed()
                        {
                            export.motion_blur = if blur <= 1 { None } else { Some(blur) };
                            changed = true;
                        }
                        ui.end_row();

                        ui.label("Output");
                        ui.label(
                            egui::RichText::new(export.output.display().to_string())
                                .weak()
                                .size(11.0),
                        )
                        .on_hover_text("Click to change")
                        .clicked()
                        .then(|| {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                export.output = dir;
                                changed = true;
                            }
                        });
                        ui.end_row();
                    });
            });

            if changed {
                doc.dirty = true;
                app.preview_dirty = true;
            }
        });
}
