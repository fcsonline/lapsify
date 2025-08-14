use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Lapsify GUI",
        options,
        Box::new(|_cc| {
            Ok(Box::<LapsifyApp>::default())
        }),
    )
}

#[derive(Default)]
struct LapsifyApp {
    // Placeholder for now - will be expanded in later tasks
}

impl eframe::App for LapsifyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Lapsify GUI");
            ui.label("Time-lapse image processor with adjustable parameters");
            ui.separator();
            ui.label("GUI application is being built...");
        });
    }
}