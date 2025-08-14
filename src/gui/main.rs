use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use std::time::SystemTime;
use image::GenericImageView;

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

// Core data structures for state management

/// Main application state containing all GUI state
#[derive(Default)]
pub struct AppState {
    pub selected_folder: Option<PathBuf>,
    pub images: Vec<ImageInfo>,
    pub selected_image_index: Option<usize>,
    pub settings: LapsifySettings,
    pub processing_status: ProcessingStatus,
    pub ui_state: UiState,
}

impl AppState {
    /// Create a new AppState with default values
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set the selected folder and clear existing images
    pub fn set_selected_folder(&mut self, folder: PathBuf) {
        self.selected_folder = Some(folder);
        self.images.clear();
        self.selected_image_index = None;
    }
    
    /// Validate that the selected folder exists and is readable
    pub fn validate_selected_folder(&self) -> Result<(), String> {
        match &self.selected_folder {
            Some(folder) => {
                if !folder.exists() {
                    return Err(format!("Folder does not exist: {}", folder.display()));
                }
                if !folder.is_dir() {
                    return Err(format!("Path is not a directory: {}", folder.display()));
                }
                // Try to read the directory to check permissions
                match std::fs::read_dir(folder) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(format!("Cannot read directory {}: {}", folder.display(), e)),
                }
            }
            None => Err("No folder selected".to_string()),
        }
    }
    
    /// Scan the selected folder for supported image files
    pub fn scan_images(&mut self) -> Result<usize, String> {
        let folder = match &self.selected_folder {
            Some(folder) => folder,
            None => return Err("No folder selected".to_string()),
        };
        
        // Clear existing images
        self.images.clear();
        self.selected_image_index = None;
        
        // Read directory and collect image files
        let entries = fs::read_dir(folder)
            .map_err(|e| format!("Failed to read directory: {}", e))?;
        
        let mut image_paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| is_image_file(path))
            .collect();
        
        // Sort chronologically by modification time, fallback to filename
        image_paths.sort_by(|a, b| {
            let a_time = get_file_modified_time(a);
            let b_time = get_file_modified_time(b);
            
            match (a_time, b_time) {
                (Some(a_time), Some(b_time)) => a_time.cmp(&b_time),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.file_name().cmp(&b.file_name()),
            }
        });
        
        // Create ImageInfo objects for each image
        for path in image_paths {
            let metadata = create_image_metadata(&path);
            let image_info = ImageInfo {
                path,
                thumbnail: None,
                full_image: None,
                metadata,
            };
            self.images.push(image_info);
        }
        
        // Select the first image if any were found
        if !self.images.is_empty() {
            self.selected_image_index = Some(0);
        }
        
        Ok(self.images.len())
    }
    
    /// Add an image to the collection
    pub fn add_image(&mut self, image_info: ImageInfo) {
        self.images.push(image_info);
    }
    
    /// Select an image by index
    pub fn select_image(&mut self, index: usize) {
        if index < self.images.len() {
            self.selected_image_index = Some(index);
        }
    }
    
    /// Get the currently selected image
    pub fn get_selected_image(&self) -> Option<&ImageInfo> {
        self.selected_image_index
            .and_then(|index| self.images.get(index))
    }
    
    /// Update processing status
    pub fn update_processing_status(&mut self, status: ProcessingStatus) {
        self.processing_status = status;
    }
    
    /// Validate current settings and update UI validation state
    pub fn validate_settings(&mut self) {
        self.ui_state.validation_errors = self.settings.validate();
    }
}

/// Information about a loaded image including metadata and texture handles
#[derive(Clone)]
pub struct ImageInfo {
    pub path: PathBuf,
    pub thumbnail: Option<egui::TextureHandle>,
    pub full_image: Option<egui::TextureHandle>,
    pub metadata: ImageMetadata,
}

/// Image metadata for display and processing
#[derive(Clone, Default)]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub format: String,
    pub modified: Option<std::time::SystemTime>,
}

/// Settings struct mirroring CLI parameters from main.rs
#[derive(Clone, Serialize, Deserialize)]
pub struct LapsifySettings {
    // Image adjustments - support for single values or arrays for animation
    pub exposure: Vec<f32>,
    pub brightness: Vec<f32>,
    pub contrast: Vec<f32>,
    pub saturation: Vec<f32>,
    
    // Crop and positioning
    pub crop: Option<String>,
    pub offset_x: Vec<f32>,
    pub offset_y: Vec<f32>,
    
    // Output settings
    pub format: String,
    pub fps: u32,
    pub quality: u32,
    pub resolution: Option<String>,
    
    // Processing settings
    pub threads: usize,
    pub start_frame: Option<usize>,
    pub end_frame: Option<usize>,
}

impl Default for LapsifySettings {
    fn default() -> Self {
        Self {
            // Default values matching CLI defaults from main.rs
            exposure: vec![0.0],     // EV stops (+/- values)
            brightness: vec![0.0],   // -100 to +100
            contrast: vec![1.0],     // 0.0 to 2.0 (1.0 = no change)
            saturation: vec![1.0],   // 0.0 to 2.0 (1.0 = no change)
            crop: None,              // Crop string in format "width:height:x:y"
            offset_x: vec![0.0],     // X offset for crop window (pixels)
            offset_y: vec![0.0],     // Y offset for crop window (pixels)
            format: "mp4".to_string(), // Default output format
            fps: 24,                 // Default frame rate
            quality: 20,             // Default CRF quality
            resolution: None,        // Default: original size
            threads: 0,              // 0 = auto-detect
            start_frame: None,       // Default: start from beginning
            end_frame: None,         // Default: process to end
        }
    }
}

impl LapsifySettings {
    /// Save settings to a JSON file
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
    
    /// Load settings from a JSON file
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let settings: LapsifySettings = serde_json::from_str(&json)?;
        Ok(settings)
    }
    
    /// Validate all settings parameters according to CLI constraints
    pub fn validate(&self) -> HashMap<String, String> {
        let mut errors = HashMap::new();
        
        // Validate exposure values (-3.0 to +3.0)
        for (i, &value) in self.exposure.iter().enumerate() {
            if value < -3.0 || value > 3.0 {
                errors.insert(
                    format!("exposure[{}]", i),
                    format!("Exposure value {} is outside valid range [-3.0, 3.0]", value)
                );
            }
        }
        
        // Validate brightness values (-100 to +100)
        for (i, &value) in self.brightness.iter().enumerate() {
            if value < -100.0 || value > 100.0 {
                errors.insert(
                    format!("brightness[{}]", i),
                    format!("Brightness value {} is outside valid range [-100.0, 100.0]", value)
                );
            }
        }
        
        // Validate contrast values (0.1 to 3.0)
        for (i, &value) in self.contrast.iter().enumerate() {
            if value < 0.1 || value > 3.0 {
                errors.insert(
                    format!("contrast[{}]", i),
                    format!("Contrast value {} is outside valid range [0.1, 3.0]", value)
                );
            }
        }
        
        // Validate saturation values (0.0 to 2.0)
        for (i, &value) in self.saturation.iter().enumerate() {
            if value < 0.0 || value > 2.0 {
                errors.insert(
                    format!("saturation[{}]", i),
                    format!("Saturation value {} is outside valid range [0.0, 2.0]", value)
                );
            }
        }
        
        // Validate FPS (1 to 120)
        if self.fps < 1 || self.fps > 120 {
            errors.insert(
                "fps".to_string(),
                format!("FPS {} is outside valid range [1, 120]", self.fps)
            );
        }
        
        // Validate quality/CRF (0 to 51)
        if self.quality > 51 {
            errors.insert(
                "quality".to_string(),
                format!("Quality (CRF) {} is outside valid range [0, 51]", self.quality)
            );
        }
        
        // Validate frame range
        if let (Some(start), Some(end)) = (self.start_frame, self.end_frame) {
            if start > end {
                errors.insert(
                    "frame_range".to_string(),
                    "Start frame must be less than or equal to end frame".to_string()
                );
            }
        }
        
        // Validate format
        let valid_formats = ["mp4", "mov", "avi", "jpg", "png", "tiff"];
        if !valid_formats.contains(&self.format.as_str()) {
            errors.insert(
                "format".to_string(),
                format!("Format '{}' is not supported. Valid formats: {}", 
                    self.format, valid_formats.join(", "))
            );
        }
        
        errors
    }
}

/// Processing status for tracking time-lapse generation
#[derive(Default, Clone)]
pub struct ProcessingStatus {
    pub is_processing: bool,
    pub progress: f32,
    pub current_frame: usize,
    pub total_frames: usize,
    pub status_message: String,
    pub error_message: Option<String>,
    pub output_path: Option<PathBuf>,
}

/// UI state for managing interface elements
pub struct UiState {
    pub sidebar_width: f32,
    pub carousel_height: f32,
    pub show_settings_validation: bool,
    pub validation_errors: HashMap<String, String>,
    pub zoom_level: f32,
    pub pan_offset: egui::Vec2,
    pub folder_error: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_width: 300.0,
            carousel_height: 150.0,
            show_settings_validation: true,
            validation_errors: HashMap::new(),
            zoom_level: 1.0,
            pan_offset: egui::Vec2::ZERO,
            folder_error: None,
        }
    }
}

/// Check if a file is a supported image format (matching lapsify CLI)
fn is_image_file(path: &Path) -> bool {
    if let Some(extension) = path.extension() {
        if let Some(ext_str) = extension.to_str() {
            matches!(
                ext_str.to_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "tiff" | "tif" | "bmp" | "webp" | "raw" | "cr2" | "nef" | "arw"
            )
        } else {
            false
        }
    } else {
        false
    }
}

/// Get file modification time, returning None if unavailable
fn get_file_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
}

/// Create image metadata from file path
fn create_image_metadata(path: &Path) -> ImageMetadata {
    let mut metadata = ImageMetadata::default();
    
    // Get file size and modification time
    if let Ok(file_metadata) = fs::metadata(path) {
        metadata.file_size = file_metadata.len();
        metadata.modified = file_metadata.modified().ok();
    }
    
    // Determine format from extension
    if let Some(extension) = path.extension() {
        if let Some(ext_str) = extension.to_str() {
            metadata.format = match ext_str.to_lowercase().as_str() {
                "jpg" | "jpeg" => "JPEG".to_string(),
                "png" => "PNG".to_string(),
                "tiff" | "tif" => "TIFF".to_string(),
                "bmp" => "BMP".to_string(),
                "webp" => "WebP".to_string(),
                "raw" | "cr2" | "nef" | "arw" => "RAW".to_string(),
                _ => ext_str.to_uppercase(),
            };
        }
    }
    
    // Try to get image dimensions using the image crate
    // This is done lazily to avoid blocking the UI
    if let Ok(img) = image::open(path) {
        let (width, height) = img.dimensions();
        metadata.width = width;
        metadata.height = height;
    }
    
    metadata
}

struct LapsifyApp {
    state: AppState,
    initialized: bool,
}

impl Default for LapsifyApp {
    fn default() -> Self {
        Self {
            state: AppState::default(),
            initialized: false,
        }
    }
}

impl LapsifyApp {
    /// Initialize with some test data for demonstration
    fn init_test_data(&mut self) {
        // Add some mock images for testing the layout
        use std::path::PathBuf;
        
        self.state.set_selected_folder(PathBuf::from("/test/folder"));
        
        for i in 1..=8 {
            let image_info = ImageInfo {
                path: PathBuf::from(format!("/test/folder/image_{:03}.jpg", i)),
                thumbnail: None,
                full_image: None,
                metadata: ImageMetadata {
                    width: 1920,
                    height: 1080,
                    file_size: 2_500_000,
                    format: "JPEG".to_string(),
                    modified: None,
                },
            };
            self.state.add_image(image_info);
        }
        
        // Select the first image
        if !self.state.images.is_empty() {
            self.state.select_image(0);
        }
    }
    
    /// Handle folder selection using file dialog
    fn select_folder(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Select Image Folder")
            .pick_folder()
        {
            // Clear any previous folder error
            self.state.ui_state.folder_error = None;
            
            // Set the selected folder
            self.state.set_selected_folder(folder.clone());
            
            // Validate the selected folder
            match self.state.validate_selected_folder() {
                Ok(()) => {
                    // Folder is valid, now scan for images
                    match self.state.scan_images() {
                        Ok(count) => {
                            // Successfully scanned images
                            self.state.ui_state.folder_error = None;
                            println!("Scanned {} images from {}", count, folder.display());
                        }
                        Err(error) => {
                            // Error scanning images
                            self.state.ui_state.folder_error = Some(format!("Error scanning images: {}", error));
                        }
                    }
                }
                Err(error) => {
                    // Store the validation error for display
                    self.state.ui_state.folder_error = Some(error);
                }
            }
        }
    }
    
    /// Manually refresh/rescan the current folder
    fn refresh_images(&mut self) {
        if self.state.selected_folder.is_some() {
            match self.state.scan_images() {
                Ok(count) => {
                    self.state.ui_state.folder_error = None;
                    println!("Refreshed: found {} images", count);
                }
                Err(error) => {
                    self.state.ui_state.folder_error = Some(format!("Error refreshing images: {}", error));
                }
            }
        }
    }

    /// Display the settings sidebar panel
    fn show_settings_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.separator();
        
        // Folder selection section
        ui.heading("Input Folder");
        
        // Folder selection and refresh buttons
        ui.horizontal(|ui| {
            if ui.button("📁 Select Folder").clicked() {
                self.select_folder();
            }
            
            // Show refresh button only if a folder is selected
            if self.state.selected_folder.is_some() {
                if ui.button("🔄 Refresh").clicked() {
                    self.refresh_images();
                }
            }
        });
        
        // Display selected folder path and image count
        if let Some(folder) = &self.state.selected_folder {
            ui.horizontal(|ui| {
                ui.label("Selected:");
                ui.label(folder.display().to_string());
            });
            
            // Show image count and status
            if self.state.ui_state.folder_error.is_none() {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().selection.bg_fill, "✓");
                    ui.label(format!("{} images found", self.state.images.len()));
                    
                    // Show supported formats info
                    if ui.small_button("ℹ").clicked() {
                        // This could open a tooltip or info dialog
                    }
                });
                
                // Show some basic stats if images are loaded
                if !self.state.images.is_empty() {
                    ui.collapsing("Image Details", |ui| {
                        if let Some(selected_image) = self.state.get_selected_image() {
                            ui.label(format!("Selected: {}", 
                                selected_image.path.file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()));
                            ui.label(format!("Format: {}", selected_image.metadata.format));
                            ui.label(format!("Size: {}x{}", 
                                selected_image.metadata.width, 
                                selected_image.metadata.height));
                            ui.label(format!("File size: {:.1} MB", 
                                selected_image.metadata.file_size as f64 / 1_048_576.0));
                        }
                    });
                }
            }
        } else {
            ui.label("No folder selected");
        }
        
        // Display folder validation or scanning error if any
        if let Some(error) = &self.state.ui_state.folder_error {
            ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {}", error));
        }
        
        ui.separator();
        
        // Test data controls (for development)
        ui.collapsing("Development Tools", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Load Test Data").clicked() {
                    self.init_test_data();
                }
                if ui.button("Clear Data").clicked() {
                    self.state.images.clear();
                    self.state.selected_image_index = None;
                    self.state.selected_folder = None;
                    self.state.ui_state.folder_error = None;
                }
            });
        });
        ui.separator();
        
        // Placeholder content for settings panel
        ui.label("Lapsify Parameters");
        ui.separator();
        
        ui.collapsing("Image Adjustments", |ui| {
            ui.label("• Exposure");
            ui.label("• Brightness");
            ui.label("• Contrast");
            ui.label("• Saturation");
        });
        
        ui.collapsing("Output Settings", |ui| {
            ui.label("• Format");
            ui.label("• FPS");
            ui.label("• Quality");
            ui.label("• Resolution");
        });
        
        ui.collapsing("Processing", |ui| {
            ui.label("• Threads");
            ui.label("• Frame Range");
            ui.label("• Crop Settings");
        });
        
        ui.separator();
        ui.label(format!("Panel width: {:.0}px", ui.available_width()));
    }
    
    /// Display the thumbnail carousel panel
    fn show_thumbnail_carousel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Image Carousel");
        ui.separator();
        
        // Show folder status and image count
        match &self.state.selected_folder {
            Some(folder) => {
                ui.horizontal(|ui| {
                    ui.label("📁");
                    ui.label(folder.file_name().unwrap_or_default().to_string_lossy());
                    ui.label(format!("({} images)", self.state.images.len()));
                    
                    // Show chronological info if images are loaded
                    if !self.state.images.is_empty() {
                        if let (Some(first), Some(last)) = (self.state.images.first(), self.state.images.last()) {
                            if let (Some(first_time), Some(last_time)) = 
                                (first.metadata.modified, last.metadata.modified) {
                                let duration = last_time.duration_since(first_time).unwrap_or_default();
                                ui.label(format!("(span: {:.1}h)", duration.as_secs_f64() / 3600.0));
                            }
                        }
                    }
                });
            }
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label("No folder selected. Click 'Select Folder' to begin.");
                });
                return;
            }
        }
        
        ui.separator();
        
        // Placeholder content for carousel
        if self.state.images.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No images found in selected folder.");
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(format!("{} images loaded", self.state.images.len()));
                if let Some(index) = self.state.selected_image_index {
                    ui.label(format!("Selected: {}", index + 1));
                }
            });
            
            // Placeholder for thumbnail strip
            ui.separator();
            egui::ScrollArea::horizontal()
                .id_source("thumbnail_scroll")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for i in 0..self.state.images.len() {
                            let is_selected = self.state.selected_image_index == Some(i);
                            let button_text = format!("Img {}", i + 1);
                            
                            let button = if is_selected {
                                egui::Button::new(&button_text).fill(ui.visuals().selection.bg_fill)
                            } else {
                                egui::Button::new(&button_text)
                            };
                            
                            if ui.add_sized([80.0, 60.0], button).clicked() {
                                self.state.select_image(i);
                            }
                        }
                    });
                });
        }
        
        ui.separator();
        ui.label(format!("Panel height: {:.0}px", ui.available_height()));
    }
    
    /// Display the main image viewer panel
    fn show_main_viewer(&mut self, ui: &mut egui::Ui) {
        ui.heading("Image Viewer");
        ui.separator();
        
        // Display current folder info and status
        match &self.state.selected_folder {
            Some(folder) => {
                ui.horizontal(|ui| {
                    ui.label("📁 Folder:");
                    ui.label(folder.display().to_string());
                });
                
                // Show folder validation status and image scanning results
                if let Some(error) = &self.state.ui_state.folder_error {
                    ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {}", error));
                } else {
                    ui.horizontal(|ui| {
                        ui.colored_label(ui.visuals().selection.bg_fill, "✓ Folder loaded");
                        ui.label(format!("({} images found)", self.state.images.len()));
                        
                        if !self.state.images.is_empty() {
                            // Show format breakdown
                            let mut format_counts: HashMap<String, usize> = HashMap::new();
                            for image in &self.state.images {
                                *format_counts.entry(image.metadata.format.clone()).or_insert(0) += 1;
                            }
                            
                            let formats: Vec<String> = format_counts.iter()
                                .map(|(format, count)| format!("{}({})", format, count))
                                .collect();
                            
                            if !formats.is_empty() {
                                ui.label(format!("- {}", formats.join(", ")));
                            }
                        }
                    });
                }
                ui.separator();
            }
            None => {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.label("No folder selected");
                        ui.label("Click 'Select Folder' in the sidebar to begin");
                        if ui.button("📁 Select Folder").clicked() {
                            self.select_folder();
                        }
                    });
                });
                return; // Exit early if no folder is selected
            }
        }
        
        // Main viewer area
        let available_rect = ui.available_rect_before_wrap();
        
        if let Some(selected_image) = self.state.get_selected_image() {
            // Display selected image info
            ui.horizontal(|ui| {
                ui.label("Selected:");
                ui.label(selected_image.path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy());
            });
            
            ui.horizontal(|ui| {
                ui.label(format!("Size: {}x{}", 
                    selected_image.metadata.width, 
                    selected_image.metadata.height));
                ui.label(format!("Format: {}", selected_image.metadata.format));
            });
            
            // Placeholder for actual image display
            let image_rect = egui::Rect::from_min_size(
                available_rect.min + egui::vec2(10.0, 80.0),
                available_rect.size() - egui::vec2(20.0, 90.0)
            );
            
            ui.painter().rect_stroke(
                image_rect,
                5.0,
                egui::Stroke::new(2.0, ui.visuals().text_color())
            );
            
            ui.painter().text(
                image_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Image will be displayed here",
                egui::FontId::proportional(16.0),
                ui.visuals().text_color()
            );
            
        } else {
            // No image selected placeholder
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.label("No image selected");
                    ui.label("Click on a thumbnail to view an image");
                });
            });
        }
        
        // Display panel dimensions for debugging
        ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
            ui.label(format!("Viewer size: {:.0}x{:.0}px", 
                available_rect.width(), available_rect.height()));
        });
    }
}

impl eframe::App for LapsifyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Initialize test data on first run
        if !self.initialized {
            self.init_test_data();
            self.initialized = true;
        }
        
        // Left sidebar panel for settings
        let sidebar_response = egui::SidePanel::left("settings_sidebar")
            .resizable(true)
            .default_width(self.state.ui_state.sidebar_width)
            .width_range(250.0..=400.0)
            .show(ctx, |ui| {
                self.show_settings_sidebar(ui);
            });
        
        // Update stored sidebar width if it was resized
        self.state.ui_state.sidebar_width = sidebar_response.response.rect.width();

        // Bottom panel for thumbnail carousel
        let carousel_response = egui::TopBottomPanel::bottom("thumbnail_carousel")
            .resizable(true)
            .default_height(self.state.ui_state.carousel_height)
            .height_range(100.0..=250.0)
            .show(ctx, |ui| {
                self.show_thumbnail_carousel(ui);
            });
        
        // Update stored carousel height if it was resized
        self.state.ui_state.carousel_height = carousel_response.response.rect.height();

        // Central panel for main image viewer
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_main_viewer(ui);
        });
    }
}