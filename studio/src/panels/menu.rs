//! Application menu bar: File / Edit / Analyze / View / Help.

use crate::app::StudioApp;

#[cfg(not(target_os = "macos"))]
pub const SHORTCUT_OPEN: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
#[cfg(not(target_os = "macos"))]
pub const SHORTCUT_SAVE: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);

#[cfg(not(target_os = "macos"))]
pub fn show(app: &mut StudioApp, ctx: &egui::Context) {
    // Global shortcuts work whether or not a menu is open.
    if ctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_SAVE)) {
        app.save();
    }
    if ctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_OPEN)) {
        open_project_dialog(app);
    }

    egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            let has_doc = app.doc.is_some();
            let idle = app.worker.job_running.is_none() && has_doc;

            ui.menu_button("File", |ui| {
                if ui.button("Open folder…").clicked() {
                    ui.close();
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        app.open_folder(&dir);
                    }
                }
                if ui
                    .add(
                        egui::Button::new("Open project…")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_OPEN)),
                    )
                    .clicked()
                {
                    ui.close();
                    open_project_dialog(app);
                }
                ui.separator();
                if ui
                    .add_enabled(
                        has_doc,
                        egui::Button::new("Save")
                            .shortcut_text(ctx.format_shortcut(&SHORTCUT_SAVE)),
                    )
                    .clicked()
                {
                    ui.close();
                    app.save();
                }
                if ui
                    .add_enabled(has_doc, egui::Button::new("Save as…"))
                    .clicked()
                {
                    ui.close();
                    app.save_as();
                }
                ui.separator();
                if ui
                    .add_enabled(has_doc, egui::Button::new("Reveal frames folder"))
                    .clicked()
                {
                    ui.close();
                    app.reveal_frames_folder();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.close();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(has_doc, egui::Button::new("Reset all adjustments"))
                    .clicked()
                {
                    ui.close();
                    app.reset_adjustments();
                }
                if ui
                    .add_enabled(has_doc, egui::Button::new("Reset current curve"))
                    .clicked()
                {
                    ui.close();
                    app.reset_current_curve();
                }
                ui.separator();
                if ui
                    .add_enabled(has_doc, egui::Button::new("Clear analysis layers"))
                    .on_hover_text(
                        "Remove measured luminance, EXIF compensation and deflicker data",
                    )
                    .clicked()
                {
                    ui.close();
                    app.clear_analysis();
                }
            });

            ui.menu_button("Analyze", |ui| {
                if ui
                    .add_enabled(idle, egui::Button::new("Measure luminance"))
                    .clicked()
                {
                    ui.close();
                    app.job_luminance(false);
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Measure developed luminance"))
                    .clicked()
                {
                    ui.close();
                    app.job_luminance(true);
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Compensate EXIF exposure"))
                    .clicked()
                {
                    ui.close();
                    app.job_holygrail();
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Deflicker"))
                    .clicked()
                {
                    ui.close();
                    app.job_deflicker();
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Suggest keyframes"))
                    .clicked()
                {
                    ui.close();
                    app.job_suggest_keyframes();
                }
                ui.separator();
                if ui.add_enabled(idle, egui::Button::new("Export…")).clicked() {
                    ui.close();
                    app.job_render();
                }
            });

            ui.menu_button("View", |ui| {
                ui.label(egui::RichText::new("Timeline layers").weak());
                ui.checkbox(&mut app.show_source_luma, "Source luminance");
                ui.checkbox(&mut app.show_developed_luma, "Developed luminance");
                ui.checkbox(&mut app.show_compensation, "EXIF compensation");
                ui.checkbox(&mut app.show_deflicker, "Deflicker");
            });

            ui.menu_button("Help", |ui| {
                if ui.button("About Lapsify Studio").clicked() {
                    ui.close();
                    app.show_about = true;
                }
            });
        });
    });

    if app.show_about {
        show_about_window(app, ctx);
    }
}

#[cfg(not(target_os = "macos"))]
fn open_project_dialog(app: &mut StudioApp) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Lapsify project", &["json"])
        .pick_file()
    {
        app.open_project(&path);
    }
}

pub fn show_about_window(app: &mut StudioApp, ctx: &egui::Context) {
    let mut open = app.show_about;
    egui::Window::new("About Lapsify Studio")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                if let Some(logo) = &app.logo {
                    ui.add(egui::Image::new((logo.id(), egui::vec2(72.0, 72.0))));
                    ui.add_space(8.0);
                }
                ui.label(egui::RichText::new("Lapsify Studio").strong().size(18.0));
                ui.label(
                    egui::RichText::new(format!("version {}", env!("CARGO_PKG_VERSION"))).weak(),
                );
                ui.add_space(6.0);
                ui.label("Visual editor for lapsify time-lapse projects.");
                ui.label(
                    egui::RichText::new(
                        "Everything you edit here is stored in the same project.json the CLI uses.",
                    )
                    .weak()
                    .size(11.5),
                );
                ui.add_space(8.0);
                ui.hyperlink_to(
                    "github.com/fcsonline/lapsify",
                    "https://github.com/fcsonline/lapsify",
                );
                ui.add_space(8.0);
            });
        });
    app.show_about = open;
}
