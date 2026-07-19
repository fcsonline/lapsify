#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod document;
mod panels;
mod theme;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_maximized(true)
            .with_title("Lapsify Studio"),
        ..Default::default()
    };
    // Optional startup document: a project.json or a frames folder.
    let open_on_start = std::env::args().nth(1).map(std::path::PathBuf::from);

    eframe::run_native(
        "Lapsify Studio",
        options,
        Box::new(move |cc| {
            let mut app = app::StudioApp::new(cc);
            if let Some(path) = open_on_start {
                if path.is_dir() {
                    app.open_folder(&path);
                } else {
                    app.open_project(&path);
                }
            }
            Ok(Box::new(app))
        }),
    )
}
