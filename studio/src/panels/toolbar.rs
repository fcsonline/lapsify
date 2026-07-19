use crate::app::StudioApp;

pub fn show(app: &mut StudioApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

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
                .add_enabled(has_doc, egui::Button::new(if dirty { "💾 Save*" } else { "💾 Save" }))
                .clicked()
            {
                app.save();
            }

            ui.separator();

            let idle = app.worker.job_running.is_none() && has_doc;
            if ui
                .add_enabled(idle, egui::Button::new("☀ Luminance"))
                .on_hover_text("Measure per-frame brightness (needed for the curve graph and keyframe suggestions)")
                .clicked()
            {
                app.job_luminance(false);
            }
            if ui
                .add_enabled(idle, egui::Button::new("📷 Compensate EXIF"))
                .on_hover_text("Cancel in-camera exposure jumps using shutter/aperture/ISO metadata")
                .clicked()
            {
                app.job_holygrail();
            }
            if ui
                .add_enabled(idle, egui::Button::new("〰 Deflicker"))
                .on_hover_text("Smooth developed luminance into a target and correct every frame toward it")
                .clicked()
            {
                app.job_deflicker();
            }
            if ui
                .add_enabled(idle, egui::Button::new("🔑 Suggest keyframes"))
                .on_hover_text("Place exposure keyframes where brightness actually changes")
                .clicked()
            {
                app.job_suggest_keyframes();
            }

            ui.separator();

            if ui
                .add_enabled(idle, egui::Button::new("🎬 Render"))
                .on_hover_text("Render the full sequence with the current settings")
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
        });
    });
}
