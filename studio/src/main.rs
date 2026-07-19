#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod document;
mod native_menu;
mod panels;
mod theme;
mod worker;

/// The official mark, decoded for the window/dock icon.
fn app_icon() -> egui::IconData {
    match image::load_from_memory(include_bytes!("../assets/icon-256.png")) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            }
        }
        Err(_) => egui::IconData::default(),
    }
}

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([900.0, 600.0])
        .with_maximized(true)
        .with_icon(app_icon())
        .with_title("Lapsify Studio");
    // Seamless mac chrome: content extends under a transparent titlebar and
    // the traffic lights float over the toolbar.
    #[cfg(target_os = "macos")]
    {
        viewport = viewport
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_fullsize_content_view(true);
    }
    let options = eframe::NativeOptions {
        viewport,
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
