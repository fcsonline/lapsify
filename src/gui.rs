use eframe::egui;
use rfd::FileDialog;
use std::path::{Path, PathBuf};
use std::fs;
use std::sync::{Arc, Mutex};

// Import the existing Lapsify processing logic
use lapsify::{ImageAdjustments, apply_adjustments, is_image_file};

#[derive(Clone)]
struct LapsifyParameters {
    exposure: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    crop_enabled: bool,
    crop_width: f32,
    crop_height: f32,
    crop_x: f32,
    crop_y: f32,
    offset_x: f32,
    offset_y: f32,
}

impl Default for LapsifyParameters {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            crop_enabled: false,
            crop_width: 100.0,
            crop_height: 100.0,
            crop_x: 0.0,
            crop_y: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

struct LapsifyGUI {
    image_list: Vec<PathBuf>,
    current_image_index: usize,
    parameters: Arc<Mutex<LapsifyParameters>>,
    status_message: String,
    selected_folder: Option<PathBuf>,
    current_texture_id: Option<egui::TextureId>,
    texture_size: [u32; 2],
    needs_image_update: bool,
}

impl Default for LapsifyGUI {
    fn default() -> Self {
        Self {
            image_list: Vec::new(),
            current_image_index: 0,
            parameters: Arc::new(Mutex::new(LapsifyParameters::default())),
            status_message: "Ready".to_string(),
            selected_folder: None,
            current_texture_id: None,
            texture_size: [0, 0],
            needs_image_update: false,
        }
    }
}

impl eframe::App for LapsifyGUI {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Main layout with top panel, central area, and bottom carousel
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Lapsify - Time-lapse Processor");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&self.status_message);
                });
            });
        });

        egui::TopBottomPanel::bottom("carousel").show(ctx, |ui| {
            self.show_carousel(ui);
        });

        egui::CentralPanel::default().show(ctx, |_ui| {
            // Main content area with left and right panels
            egui::SidePanel::left("folder_panel").show(ctx, |ui| {
                self.show_folder_panel(ui);
            });

            egui::SidePanel::right("parameters").show(ctx, |ui| {
                self.show_parameters_panel(ui);
            });

            // Center content area
            egui::CentralPanel::default().show(ctx, |ui| {
                self.show_main_content(ui);
            });
        });

        // Handle keyboard shortcuts
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            self.previous_image();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.next_image();
        }

        // Update image if needed
        if self.needs_image_update {
            self.update_current_image(ctx);
            self.needs_image_update = false;
        }
    }
}

impl LapsifyGUI {
    fn show_folder_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Folder Selection");
        ui.separator();

        // Show current folder
        if let Some(folder) = &self.selected_folder {
            ui.label("Current Folder:");
            ui.label(folder.to_string_lossy());
        } else {
            ui.label("No folder selected");
        }

        ui.separator();

        // Folder selection button
        if ui.button("📁 Select Image Folder").clicked() {
            self.select_folder();
        }

        if !self.image_list.is_empty() {
            ui.separator();
            ui.label(format!("Images loaded: {}", self.image_list.len()));
            
            // Navigation buttons
            ui.horizontal(|ui| {
                if ui.button("◀ Previous").clicked() {
                    self.previous_image();
                }
                
                if ui.button("Next ▶").clicked() {
                    self.next_image();
                }
            });
        }
    }

    fn show_parameters_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Parameters");
        
        // Handle parameter changes
        let mut needs_update = false;
        
        // Get current parameters
        let mut params = self.parameters.lock().unwrap();
        
        // Exposure
        ui.label("Exposure");
        let mut exposure = params.exposure;
        if ui.add(egui::Slider::new(&mut exposure, -3.0..=3.0).text("EV")).changed() {
            params.exposure = exposure;
            needs_update = true;
        }
        
        // Brightness
        ui.label("Brightness");
        let mut brightness = params.brightness;
        if ui.add(egui::Slider::new(&mut brightness, -100.0..=100.0)).changed() {
            params.brightness = brightness;
            needs_update = true;
        }
        
        // Contrast
        ui.label("Contrast");
        let mut contrast = params.contrast;
        if ui.add(egui::Slider::new(&mut contrast, 0.1..=3.0)).changed() {
            params.contrast = contrast;
            needs_update = true;
        }
        
        // Saturation
        ui.label("Saturation");
        let mut saturation = params.saturation;
        if ui.add(egui::Slider::new(&mut saturation, 0.0..=2.0)).changed() {
            params.saturation = saturation;
            needs_update = true;
        }
        
        ui.separator();
        
        // Crop settings
        ui.heading("Crop Settings");
        let mut crop_enabled = params.crop_enabled;
        if ui.checkbox(&mut crop_enabled, "Enable Crop").changed() {
            params.crop_enabled = crop_enabled;
            needs_update = true;
        }
        
        if crop_enabled {
            ui.label("Width (%)");
            let mut crop_width = params.crop_width;
            if ui.add(egui::Slider::new(&mut crop_width, 1.0..=100.0)).changed() {
                params.crop_width = crop_width;
                needs_update = true;
            }
            
            ui.label("Height (%)");
            let mut crop_height = params.crop_height;
            if ui.add(egui::Slider::new(&mut crop_height, 1.0..=100.0)).changed() {
                params.crop_height = crop_height;
                needs_update = true;
            }
            
            ui.label("X Offset (%)");
            let mut crop_x = params.crop_x;
            if ui.add(egui::Slider::new(&mut crop_x, -100.0..=100.0)).changed() {
                params.crop_x = crop_x;
                needs_update = true;
            }
            
            ui.label("Y Offset (%)");
            let mut crop_y = params.crop_y;
            if ui.add(egui::Slider::new(&mut crop_y, -100.0..=100.0)).changed() {
                params.crop_y = crop_y;
                needs_update = true;
            }
        }
        
        ui.separator();
        
        // Frame offset
        ui.heading("Frame Offset");
        ui.label("X Offset (px)");
        let mut offset_x = params.offset_x;
        if ui.add(egui::Slider::new(&mut offset_x, -1000.0..=1000.0)).changed() {
            params.offset_x = offset_x;
            needs_update = true;
        }
        
        ui.label("Y Offset (px)");
        let mut offset_y = params.offset_y;
        if ui.add(egui::Slider::new(&mut offset_y, -1000.0..=1000.0)).changed() {
            params.offset_y = offset_y;
            needs_update = true;
        }
        
        // Release lock
        drop(params);
        
        // Update image if needed
        if needs_update {
            self.needs_image_update = true;
        }
    }

    fn show_main_content(&mut self, ui: &mut egui::Ui) {
        if !self.image_list.is_empty() {
            let current_image = &self.image_list[self.current_image_index];
            let filename = current_image.file_name().unwrap().to_str().unwrap();
            
            ui.vertical(|ui| {
                ui.heading("Image Preview");
                ui.label(format!("Current Image: {}", filename));
                ui.label(format!("Image {} of {}", self.current_image_index + 1, self.image_list.len()));
                
                // Display the image
                if let Some(texture_id) = self.current_texture_id {
                    let available_size = ui.available_size();
                    let image_size = [
                        self.texture_size[0] as f32,
                        self.texture_size[1] as f32
                    ];
                    
                    // Calculate display size to fit in available space while maintaining aspect ratio
                    let scale = (available_size.x / image_size[0]).min(available_size.y / image_size[1]).min(1.0);
                    let display_size = [
                        image_size[0] * scale,
                        image_size[1] * scale
                    ];
                    
                    ui.centered_and_justified(|ui| {
                        ui.image((texture_id, egui::vec2(display_size[0], display_size[1])));
                    });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label("Loading image...");
                    });
                }
                
                // Show current parameters
                let params = self.parameters.lock().unwrap();
                ui.separator();
                ui.heading("Current Parameters");
                ui.label(format!("Exposure: {:.2} EV", params.exposure));
                ui.label(format!("Brightness: {:.1}", params.brightness));
                ui.label(format!("Contrast: {:.2}x", params.contrast));
                ui.label(format!("Saturation: {:.2}x", params.saturation));
                
                if params.crop_enabled {
                    ui.label(format!("Crop: {}% x {}% at ({:.1}%, {:.1}%)", 
                        params.crop_width, params.crop_height, params.crop_x, params.crop_y));
                }
                
                ui.label(format!("Offset: ({:.1}px, {:.1}px)", params.offset_x, params.offset_y));
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("No images loaded");
                ui.label("Click 'Select Image Folder' to choose a folder with images");
            });
        }
    }

    fn show_carousel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Image Carousel:");
            
            if self.image_list.is_empty() {
                ui.label("No images loaded");
            } else {
                // Show current image info
                ui.label(format!("{} of {}", self.current_image_index + 1, self.image_list.len()));
                
                // Navigation buttons
                if ui.button("◀").clicked() {
                    self.previous_image();
                }
                
                if ui.button("▶").clicked() {
                    self.next_image();
                }
                
                // Show current filename
                if let Some(current_image) = self.image_list.get(self.current_image_index) {
                    if let Some(filename) = current_image.file_name() {
                        ui.label(filename.to_string_lossy());
                    }
                }
            }
        });
    }

    fn select_folder(&mut self) {
        if let Some(path) = FileDialog::new().pick_folder() {
            self.selected_folder = Some(path.clone());
            self.load_images_from_directory(&path);
        }
    }

    fn load_images_from_directory(&mut self, dir_path: &Path) {
        if !dir_path.exists() || !dir_path.is_dir() {
            self.status_message = "Invalid directory selected".to_string();
            return;
        }

        let mut image_files = Vec::new();
        if let Ok(entries) = fs::read_dir(dir_path) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let file_path = entry.path();
                    if is_image_file(&file_path) {
                        image_files.push(file_path);
                    }
                }
            }
        }

        image_files.sort();
        
        if image_files.is_empty() {
            self.status_message = "No image files found in the selected directory".to_string();
            return;
        }

        self.image_list = image_files;
        self.current_image_index = 0;
        self.status_message = format!("Loaded {} images from {}", self.image_list.len(), dir_path.to_string_lossy());
        self.needs_image_update = true;
    }

    fn update_current_image(&mut self, ctx: &egui::Context) {
        if self.image_list.is_empty() {
            return;
        }

        let image_path = &self.image_list[self.current_image_index];
        
        // Load and process the image
        if let Ok(img) = image::open(image_path) {
            // Apply current parameters
            let params = self.parameters.lock().unwrap();
            let adjustments = self.create_adjustments_from_params(&params);
            
            if let Ok(processed_img) = apply_adjustments(img, &adjustments, 0, 1) {
                // Convert to RGBA for display
                let rgba_img = processed_img.to_rgba8();
                
                // Create texture for display
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [rgba_img.width() as usize, rgba_img.height() as usize],
                    rgba_img.as_raw()
                );
                
                // Create texture ID and upload to GPU
                let texture_id = egui::TextureId::Managed(egui::Id::new("current_image").value());
                ctx.tex_manager().write().set(
                    texture_id,
                    egui::ImageDelta::full(color_image, egui::TextureFilter::Linear)
                );
                
                // Store texture info
                self.texture_size = [rgba_img.width(), rgba_img.height()];
                self.current_texture_id = Some(texture_id);
                
                // Update status
                let filename = image_path.file_name().unwrap().to_str().unwrap();
                self.status_message = format!("Image {} of {}: {} (processed)", 
                    self.current_image_index + 1, 
                    self.image_list.len(), 
                    filename);
            }
        }
    }

    fn create_adjustments_from_params(&self, params: &LapsifyParameters) -> ImageAdjustments {
        ImageAdjustments {
            exposure: vec![params.exposure],
            brightness: vec![params.brightness],
            contrast: vec![params.contrast],
            saturation: vec![params.saturation],
            crop: if params.crop_enabled {
                Some(format!("{}:{}:{}:{}", 
                    params.crop_width, params.crop_height, params.crop_x, params.crop_y))
            } else {
                None
            },
            offset_x: vec![params.offset_x],
            offset_y: vec![params.offset_y],
        }
    }

    fn next_image(&mut self) {
        if !self.image_list.is_empty() {
            self.current_image_index = (self.current_image_index + 1) % self.image_list.len();
            self.needs_image_update = true;
        }
    }

    fn previous_image(&mut self) {
        if !self.image_list.is_empty() {
            if self.current_image_index == 0 {
                self.current_image_index = self.image_list.len() - 1;
            } else {
                self.current_image_index -= 1;
            }
            self.needs_image_update = true;
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "Lapsify - Time-lapse Processor",
        options,
        Box::new(|_cc| Box::new(LapsifyGUI::default())),
    )
} 