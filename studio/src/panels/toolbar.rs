use crate::app::StudioApp;

pub fn show(app: &mut StudioApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("toolbar")
        .frame(
            egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // Leave room for the macOS traffic lights overlaying the
                // toolbar (transparent titlebar + fullsize content view).
                #[cfg(target_os = "macos")]
                ui.add_space(70.0);

                if let Some(logo) = &app.logo {
                    ui.add(egui::Image::new((logo.id(), egui::vec2(22.0, 22.0))));
                    ui.label(egui::RichText::new("Lapsify Studio").strong().size(15.0));
                    ui.separator();
                }

                if ui.button("📂 Open folder…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        app.open_folder(&dir);
                    }
                }
                if ui.button("📄 Open project…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Lapsify project", &["json"])
                        .pick_file()
                    {
                        app.open_project(&path);
                    }
                }

                let has_doc = app.doc.is_some();
                let dirty = app.doc.as_ref().is_some_and(|d| d.dirty);
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new(if dirty { "💾 Save*" } else { "💾 Save" }),
                    )
                    .clicked()
                {
                    app.save();
                }

                ui.separator();

                let idle = app.worker.job_running.is_none() && has_doc;
                if ui
                    .add_enabled(idle, egui::Button::new("🎬 Export"))
                    .on_hover_text("Render the sequence with the current export settings")
                    .clicked()
                {
                    app.job_render();
                }

                // Right side: progress / status.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(job) = app.worker.job_running {
                        if let Some((done, total)) = app.progress {
                            let fraction = if total > 0 {
                                done as f32 / total as f32
                            } else {
                                0.0
                            };
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .desired_width(180.0)
                                    .text(format!("{job} {done}/{total}")),
                            );
                        } else {
                            ui.spinner();
                            ui.label(job);
                        }
                        if let Some(note) = &app.deflicker_note {
                            ui.label(egui::RichText::new(note).weak());
                        }
                    }
                });
            });

            // The toolbar doubles as the window drag region (no titlebar).
            #[cfg(target_os = "macos")]
            {
                let response = ui.interact(
                    ui.max_rect(),
                    egui::Id::new("toolbar-window-drag"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            }
        });

    // Status / error strip.
    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if let Some(error) = &app.error {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), format!("⚠ {error}"));
                if ui.small_button("dismiss").clicked() {
                    app.error = None;
                }
            } else {
                ui.label(egui::RichText::new(&app.status).weak());
            }
            if app.doc.is_some() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(
                            "curve: double-click adds a keyframe · drag moves · right-click deletes",
                        )
                        .weak()
                        .size(11.0),
                    );
                });
            }
        });
    });
}
