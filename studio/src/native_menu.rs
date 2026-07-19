//! Native macOS menu bar via muda. On other platforms the in-window egui
//! menu bar (panels::menu) is used instead.

#[cfg(target_os = "macos")]
pub use imp::NativeMenu;

#[cfg(target_os = "macos")]
mod imp {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{
        AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem,
        Submenu,
    };

    use crate::app::StudioApp;

    pub struct NativeMenu {
        _menu: Menu,
        open_folder: MenuItem,
        open_project: MenuItem,
        save: MenuItem,
        save_as: MenuItem,
        reveal: MenuItem,
        reset_all: MenuItem,
        reset_curve: MenuItem,
        clear_analysis: MenuItem,
        luminance: MenuItem,
        developed: MenuItem,
        holygrail: MenuItem,
        deflicker: MenuItem,
        keyframes: MenuItem,
        render: MenuItem,
        view_source: CheckMenuItem,
        view_developed: CheckMenuItem,
        view_compensation: CheckMenuItem,
        view_deflicker: CheckMenuItem,
    }

    fn accel(mods: Modifiers, code: Code) -> Option<Accelerator> {
        Some(Accelerator::new(Some(mods), code))
    }

    impl NativeMenu {
        pub fn install() -> Option<Self> {
            let menu = Menu::new();

            let app_menu = Submenu::new("Lapsify Studio", true);
            let about = PredefinedMenuItem::about(
                Some("About Lapsify Studio"),
                Some(AboutMetadata {
                    name: Some("Lapsify Studio".to_string()),
                    version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    comments: Some("Visual editor for lapsify time-lapse projects".to_string()),
                    website: Some("https://github.com/fcsonline/lapsify".to_string()),
                    ..Default::default()
                }),
            );
            app_menu
                .append_items(&[
                    &about,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(None),
                ])
                .ok()?;

            let open_folder = MenuItem::new(
                "Open Folder…",
                true,
                accel(Modifiers::META | Modifiers::SHIFT, Code::KeyO),
            );
            let open_project =
                MenuItem::new("Open Project…", true, accel(Modifiers::META, Code::KeyO));
            let save = MenuItem::new("Save", true, accel(Modifiers::META, Code::KeyS));
            let save_as = MenuItem::new(
                "Save As…",
                true,
                accel(Modifiers::META | Modifiers::SHIFT, Code::KeyS),
            );
            let reveal = MenuItem::new("Reveal Frames Folder", true, None);
            let file_menu = Submenu::new("File", true);
            file_menu
                .append_items(&[
                    &open_folder,
                    &open_project,
                    &PredefinedMenuItem::separator(),
                    &save,
                    &save_as,
                    &PredefinedMenuItem::separator(),
                    &reveal,
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::close_window(None),
                ])
                .ok()?;

            let reset_all = MenuItem::new("Reset All Adjustments", true, None);
            let reset_curve = MenuItem::new("Reset Current Curve", true, None);
            let clear_analysis = MenuItem::new("Clear Analysis Layers", true, None);
            let edit_menu = Submenu::new("Edit", true);
            edit_menu
                .append_items(&[
                    &reset_all,
                    &reset_curve,
                    &PredefinedMenuItem::separator(),
                    &clear_analysis,
                ])
                .ok()?;

            let luminance = MenuItem::new(
                "Measure Luminance",
                true,
                accel(Modifiers::META, Code::KeyL),
            );
            let developed = MenuItem::new("Measure Developed Luminance", true, None);
            let holygrail = MenuItem::new("Compensate EXIF Exposure", true, None);
            let deflicker = MenuItem::new("Deflicker", true, accel(Modifiers::META, Code::KeyD));
            let keyframes = MenuItem::new(
                "Suggest Keyframes",
                true,
                accel(Modifiers::META, Code::KeyK),
            );
            let render = MenuItem::new("Export…", true, accel(Modifiers::META, Code::KeyE));
            let analyze_menu = Submenu::new("Analyze", true);
            analyze_menu
                .append_items(&[
                    &luminance,
                    &developed,
                    &holygrail,
                    &deflicker,
                    &keyframes,
                    &PredefinedMenuItem::separator(),
                    &render,
                ])
                .ok()?;

            let view_source = CheckMenuItem::new("Source Luminance", true, true, None);
            let view_developed = CheckMenuItem::new("Developed Luminance", true, false, None);
            let view_compensation = CheckMenuItem::new("EXIF Compensation", true, false, None);
            let view_deflicker = CheckMenuItem::new("Deflicker", true, false, None);
            let view_menu = Submenu::new("View", true);
            view_menu
                .append_items(&[
                    &view_source,
                    &view_developed,
                    &view_compensation,
                    &view_deflicker,
                ])
                .ok()?;

            let window_menu = Submenu::new("Window", true);
            window_menu
                .append_items(&[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::fullscreen(None),
                ])
                .ok()?;

            menu.append_items(&[
                &app_menu,
                &file_menu,
                &edit_menu,
                &analyze_menu,
                &view_menu,
                &window_menu,
            ])
            .ok()?;

            menu.init_for_nsapp();
            window_menu.set_as_windows_menu_for_nsapp();

            Some(Self {
                _menu: menu,
                open_folder,
                open_project,
                save,
                save_as,
                reveal,
                reset_all,
                reset_curve,
                clear_analysis,
                luminance,
                developed,
                holygrail,
                deflicker,
                keyframes,
                render,
                view_source,
                view_developed,
                view_compensation,
                view_deflicker,
            })
        }

        /// Poll menu events and keep item state in sync with the app.
        pub fn poll(app: &mut StudioApp) {
            let Some(menu) = app.native_menu.take() else {
                return;
            };

            while let Ok(event) = MenuEvent::receiver().try_recv() {
                menu.dispatch(&event.id, app);
            }
            menu.sync(app);

            app.native_menu = Some(menu);
        }

        fn dispatch(&self, id: &MenuId, app: &mut StudioApp) {
            match id {
                id if id == self.open_folder.id() => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        app.open_folder(&dir);
                    }
                }
                id if id == self.open_project.id() => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Lapsify project", &["json"])
                        .pick_file()
                    {
                        app.open_project(&path);
                    }
                }
                id if id == self.save.id() => app.save(),
                id if id == self.save_as.id() => app.save_as(),
                id if id == self.reveal.id() => app.reveal_frames_folder(),
                id if id == self.reset_all.id() => app.reset_adjustments(),
                id if id == self.reset_curve.id() => app.reset_current_curve(),
                id if id == self.clear_analysis.id() => app.clear_analysis(),
                id if id == self.luminance.id() => app.job_luminance(false),
                id if id == self.developed.id() => app.job_luminance(true),
                id if id == self.holygrail.id() => app.job_holygrail(),
                id if id == self.deflicker.id() => app.job_deflicker(),
                id if id == self.keyframes.id() => app.job_suggest_keyframes(),
                id if id == self.render.id() => app.job_render(),
                id if id == self.view_source.id() => {
                    app.show_source_luma = !app.show_source_luma;
                }
                id if id == self.view_developed.id() => {
                    app.show_developed_luma = !app.show_developed_luma;
                }
                id if id == self.view_compensation.id() => {
                    app.show_compensation = !app.show_compensation;
                }
                id if id == self.view_deflicker.id() => {
                    app.show_deflicker = !app.show_deflicker;
                }
                _ => {}
            }
        }

        fn sync(&self, app: &StudioApp) {
            let has_doc = app.doc.is_some();
            let idle = app.worker.job_running.is_none() && has_doc;

            for item in [&self.save, &self.save_as, &self.reveal] {
                item.set_enabled(has_doc);
            }
            for item in [&self.reset_all, &self.reset_curve, &self.clear_analysis] {
                item.set_enabled(has_doc);
            }
            for item in [
                &self.luminance,
                &self.developed,
                &self.holygrail,
                &self.deflicker,
                &self.keyframes,
                &self.render,
            ] {
                item.set_enabled(idle);
            }

            self.view_source.set_checked(app.show_source_luma);
            self.view_developed.set_checked(app.show_developed_luma);
            self.view_compensation.set_checked(app.show_compensation);
            self.view_deflicker.set_checked(app.show_deflicker);
        }
    }
}
