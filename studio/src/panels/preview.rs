use crate::app::StudioApp;

pub fn show(app: &mut StudioApp, ctx: &egui::Context) {
    let frame = egui::Frame::new()
        .fill(crate::theme::PREVIEW_BG)
        .inner_margin(10.0);
    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        let Some(doc) = &app.doc else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Open a frames folder or a project.json to start")
                        .size(18.0)
                        .weak(),
                );
            });
            return;
        };

        match &app.preview_texture {
            Some(texture) => {
                let available = ui.available_size();
                let tex_size = texture.size_vec2();
                let scale = (available.x / tex_size.x).min(available.y / tex_size.y);
                let size = tex_size * scale;
                ui.centered_and_justified(|ui| {
                    ui.add(egui::Image::new((texture.id(), size)).corner_radius(4.0));
                });
            }
            None => {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
            }
        }

        // Frame counter overlay in the corner.
        let painter = ui.painter();
        painter.text(
            ui.max_rect().left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!(
                "frame {} / {}",
                app.current_frame,
                doc.frame_count().saturating_sub(1)
            ),
            egui::FontId::monospace(13.0),
            ui.visuals().weak_text_color(),
        );
    });
}
