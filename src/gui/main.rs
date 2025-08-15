use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::fs;
use std::time::{SystemTime, Instant};
use image::{GenericImageView, DynamicImage, imageops::FilterType};
use std::thread;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::io::{BufRead, BufReader};

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

/// Session state for persistence
#[derive(Serialize, Deserialize, Default)]
pub struct SessionState {
    pub selected_folder: Option<PathBuf>,
    pub selected_image_index: Option<usize>,
    pub settings: LapsifySettings,
    pub ui_state: UiState,
}

/// Settings preset for common configurations
#[derive(Serialize, Deserialize, Clone)]
pub struct SettingsPreset {
    pub name: String,
    pub description: String,
    pub settings: LapsifySettings,
}

/// Main application state containing all GUI state
#[derive(Default)]
pub struct AppState {
    pub selected_folder: Option<PathBuf>,
    pub images: Vec<ImageInfo>,
    pub selected_image_index: Option<usize>,
    pub settings: LapsifySettings,
    pub processing_status: ProcessingStatus,
    pub ui_state: UiState,
    pub settings_presets: Vec<SettingsPreset>,
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
        
        // Clear existing images and thumbnail states
        self.images.clear();
        self.selected_image_index = None;
        self.ui_state.thumbnail_cache.clear();
        self.ui_state.thumbnail_load_states.clear();
        
        // Read directory and collect image files
        let entries = fs::read_dir(folder)
            .map_err(|e| {
                let error_msg = match e.kind() {
                    std::io::ErrorKind::NotFound => format!("Directory not found: {}", folder.display()),
                    std::io::ErrorKind::PermissionDenied => format!("Permission denied accessing directory: {}", folder.display()),
                    _ => format!("Failed to read directory {}: {}", folder.display(), e),
                };
                error_msg
            })?;
        
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
                path: path.clone(),
                thumbnail: None,
                full_image: None,
                metadata,
            };
            self.images.push(image_info);
            
            // Initialize thumbnail load state
            self.ui_state.thumbnail_load_states.insert(path, ThumbnailLoadState::NotStarted);
        }
        
        // Select the first image if any were found
        if !self.images.is_empty() {
            self.selected_image_index = Some(0);
        }
        
        Ok(self.images.len())
    }
    
    /// Request thumbnail loading for a specific image
    pub fn request_thumbnail(&mut self, image_index: usize, ctx: &egui::Context) {
        if image_index >= self.images.len() {
            return;
        }
        
        let image_path = self.images[image_index].path.clone();
        
        // Check if thumbnail is already cached
        if let Some(thumbnail) = self.ui_state.thumbnail_cache.get(&image_path) {
            self.images[image_index].thumbnail = Some(thumbnail);
            return;
        }
        
        // Check if already loading
        if let Some(ThumbnailLoadState::Loading) = self.ui_state.thumbnail_load_states.get(&image_path) {
            return;
        }
        
        // Mark as loading
        self.ui_state.thumbnail_load_states.insert(image_path.clone(), ThumbnailLoadState::Loading);
        
        // Start async thumbnail loading
        let ctx_clone = ctx.clone();
        let path_clone = image_path.clone();
        
        thread::spawn(move || {
            match load_thumbnail_async(&path_clone) {
                Ok((_color_image, _memory_size)) => {
                    // Request repaint to update UI with loaded thumbnail
                    ctx_clone.request_repaint();
                    
                    // Note: In a real implementation, we'd need a channel or shared state
                    // to communicate the loaded thumbnail back to the main thread.
                    // For now, we'll implement a simpler synchronous approach.
                }
                Err(error) => {
                    println!("Failed to load thumbnail for {}: {}", path_clone.display(), error);
                    ctx_clone.request_repaint();
                }
            }
        });
    }
    
    /// Load thumbnail synchronously (for immediate use)
    pub fn load_thumbnail_sync(&mut self, image_index: usize, ctx: &egui::Context) -> bool {
        if image_index >= self.images.len() {
            return false;
        }
        
        let image_path = self.images[image_index].path.clone();
        
        // Check if thumbnail is already cached
        if let Some(thumbnail) = self.ui_state.thumbnail_cache.get(&image_path) {
            self.images[image_index].thumbnail = Some(thumbnail);
            return true;
        }
        
        // Load thumbnail synchronously
        match load_thumbnail_async(&image_path) {
            Ok((color_image, memory_size)) => {
                // Create texture handle
                let texture = ctx.load_texture(
                    format!("thumbnail_{}", image_path.display()),
                    color_image,
                    egui::TextureOptions::LINEAR
                );
                
                // Cache the thumbnail
                self.ui_state.thumbnail_cache.insert(image_path.clone(), texture.clone(), memory_size);
                
                // Update image info
                self.images[image_index].thumbnail = Some(texture);
                
                // Update load state
                self.ui_state.thumbnail_load_states.insert(image_path, ThumbnailLoadState::Loaded);
                
                true
            }
            Err(error) => {
                println!("Failed to load thumbnail for {}: {}", image_path.display(), error);
                self.ui_state.thumbnail_load_states.insert(image_path, ThumbnailLoadState::Error(error));
                false
            }
        }
    }
    
    /// Load full-size image for viewing
    pub fn load_full_image_sync(&mut self, image_index: usize, ctx: &egui::Context) -> bool {
        if image_index >= self.images.len() {
            return false;
        }
        
        let image_path = self.images[image_index].path.clone();
        
        // Check if full image is already loaded
        if self.images[image_index].full_image.is_some() {
            return true;
        }
        
        // Load full-size image
        match load_full_image_async(&image_path) {
            Ok(color_image) => {
                // Create texture handle
                let texture = ctx.load_texture(
                    format!("full_image_{}", image_path.display()),
                    color_image,
                    egui::TextureOptions::LINEAR
                );
                
                // Update image info
                self.images[image_index].full_image = Some(texture);
                
                // Reset zoom and pan when loading new image
                self.ui_state.zoom_level = 1.0;
                self.ui_state.pan_offset = egui::Vec2::ZERO;
                
                true
            }
            Err(error) => {
                println!("Failed to load full image for {}: {}", image_path.display(), error);
                false
            }
        }
    }
    
    /// Add an image to the collection
    pub fn add_image(&mut self, image_info: ImageInfo) {
        self.images.push(image_info);
    }
    
    /// Select an image by index
    pub fn select_image(&mut self, index: usize) {
        if index < self.images.len() {
            self.selected_image_index = Some(index);
            // Queue background loading for nearby images
            self.queue_background_loading();
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
    
    /// Save session state to file
    pub fn save_session(&self) -> Result<(), String> {
        let session_state = SessionState {
            selected_folder: self.selected_folder.clone(),
            selected_image_index: self.selected_image_index,
            settings: self.settings.clone(),
            ui_state: UiState {
                sidebar_width: self.ui_state.sidebar_width,
                carousel_height: self.ui_state.carousel_height,
                show_settings_validation: self.ui_state.show_settings_validation,
                validation_errors: HashMap::new(), // Don't persist validation errors
                zoom_level: self.ui_state.zoom_level,
                pan_offset: egui::Vec2::ZERO, // Don't persist pan offset
                folder_error: None, // Don't persist errors
                thumbnail_cache: ThumbnailCache::new(100, 50), // Don't persist cache
                thumbnail_load_states: HashMap::new(), // Don't persist load states
                carousel_scroll_offset: 0.0, // Don't persist scroll
                visible_thumbnail_range: (0, 0), // Don't persist range
                output_directory: self.ui_state.output_directory.clone(),
                window_size: self.ui_state.window_size,
                window_position: self.ui_state.window_position,
                error_notifications: VecDeque::new(), // Don't persist notifications
                modal_dialog: ModalDialog::default(), // Don't persist modal state
                lapsify_cli_available: None, // Don't persist CLI check
                show_help_dialog: false, // Don't persist help dialog state
                background_load_queue: VecDeque::new(), // Don't persist load queue
                last_frame_time: None, // Don't persist frame time
            },
        };
        
        let session_dir = get_session_dir()?;
        fs::create_dir_all(&session_dir)
            .map_err(|e| format!("Failed to create session directory: {}", e))?;
        
        let session_file = session_dir.join("session.json");
        let json = serde_json::to_string_pretty(&session_state)
            .map_err(|e| format!("Failed to serialize session state: {}", e))?;
        
        fs::write(&session_file, json)
            .map_err(|e| format!("Failed to write session file: {}", e))?;
        
        Ok(())
    }
    
    /// Load session state from file
    pub fn load_session(&mut self) -> Result<(), String> {
        let session_dir = get_session_dir()?;
        let session_file = session_dir.join("session.json");
        
        if !session_file.exists() {
            return Ok(()); // No session file, use defaults
        }
        
        let json = fs::read_to_string(&session_file)
            .map_err(|e| format!("Failed to read session file: {}", e))?;
        
        let session_state: SessionState = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to deserialize session state: {}", e))?;
        
        // Restore state
        self.selected_folder = session_state.selected_folder;
        self.selected_image_index = session_state.selected_image_index;
        self.settings = session_state.settings;
        
        // Restore UI state (with runtime state reset)
        self.ui_state.sidebar_width = session_state.ui_state.sidebar_width;
        self.ui_state.carousel_height = session_state.ui_state.carousel_height;
        self.ui_state.show_settings_validation = session_state.ui_state.show_settings_validation;
        self.ui_state.zoom_level = session_state.ui_state.zoom_level;
        self.ui_state.output_directory = session_state.ui_state.output_directory;
        self.ui_state.window_size = session_state.ui_state.window_size;
        self.ui_state.window_position = session_state.ui_state.window_position;
        
        // Validate restored settings
        self.validate_settings();
        
        Ok(())
    }
    
    /// Load settings presets
    pub fn load_presets(&mut self) -> Result<(), String> {
        let session_dir = get_session_dir()?;
        let presets_file = session_dir.join("presets.json");
        
        if !presets_file.exists() {
            // Create default presets
            self.settings_presets = create_default_presets();
            self.save_presets()?;
            return Ok(());
        }
        
        let json = fs::read_to_string(&presets_file)
            .map_err(|e| format!("Failed to read presets file: {}", e))?;
        
        self.settings_presets = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to deserialize presets: {}", e))?;
        
        Ok(())
    }
    
    /// Save settings presets
    pub fn save_presets(&self) -> Result<(), String> {
        let session_dir = get_session_dir()?;
        fs::create_dir_all(&session_dir)
            .map_err(|e| format!("Failed to create session directory: {}", e))?;
        
        let presets_file = session_dir.join("presets.json");
        let json = serde_json::to_string_pretty(&self.settings_presets)
            .map_err(|e| format!("Failed to serialize presets: {}", e))?;
        
        fs::write(&presets_file, json)
            .map_err(|e| format!("Failed to write presets file: {}", e))?;
        
        Ok(())
    }
    
    /// Add an error notification to the UI
    pub fn add_error_notification(&mut self, message: String, error_type: ErrorType, auto_dismiss: bool) {
        let notification = ErrorNotification {
            message,
            error_type,
            timestamp: Instant::now(),
            auto_dismiss,
        };
        
        // Limit the number of notifications to prevent memory issues
        if self.ui_state.error_notifications.len() >= 10 {
            self.ui_state.error_notifications.pop_front();
        }
        
        self.ui_state.error_notifications.push_back(notification);
    }
    
    /// Show a modal dialog for critical errors
    pub fn show_modal_error(&mut self, title: String, message: String) {
        self.ui_state.modal_dialog = ModalDialog {
            is_open: true,
            title,
            message,
            dialog_type: DialogType::Error,
        };
    }
    
    /// Check if lapsify CLI is available and cache the result
    pub fn check_lapsify_availability(&mut self) -> bool {
        if let Some(available) = self.ui_state.lapsify_cli_available {
            return available;
        }
        
        let available = match find_lapsify_executable() {
            Ok(_) => true,
            Err(_) => false,
        };
        
        self.ui_state.lapsify_cli_available = Some(available);
        
        if !available {
            self.add_error_notification(
                "Lapsify CLI not found. Please ensure lapsify is installed and available in PATH.".to_string(),
                ErrorType::Critical,
                false,
            );
        }
        
        available
    }
    
    /// Queue images for background loading based on current selection
    pub fn queue_background_loading(&mut self) {
        if let Some(current_index) = self.selected_image_index {
            // Clear existing queue
            self.ui_state.background_load_queue.clear();
            
            // Queue current image and nearby images for loading
            let start = current_index.saturating_sub(2);
            let end = (current_index + 3).min(self.images.len());
            
            for i in start..end {
                let image_path = &self.images[i].path;
                if self.images[i].full_image.is_none() {
                    self.ui_state.background_load_queue.push_back(image_path.clone());
                }
            }
        }
    }
    
    /// Process one item from the background loading queue
    pub fn process_background_loading(&mut self, ctx: &egui::Context) -> bool {
        if let Some(path) = self.ui_state.background_load_queue.pop_front() {
            // Find the image in our list
            if let Some(image) = self.images.iter_mut().find(|img| img.path == path) {
                if image.full_image.is_none() {
                    // Load the image in background
                    match load_full_image_async(&path) {
                        Ok(color_image) => {
                            let texture = ctx.load_texture(
                                format!("full_image_{}", path.display()),
                                color_image,
                                egui::TextureOptions::default(),
                            );
                            image.full_image = Some(texture);
                            return true; // Successfully loaded
                        }
                        Err(_) => {
                            // Failed to load, skip this image
                        }
                    }
                }
            }
        }
        false // No more items to process
    }
    
    /// Clean up unused textures to free memory
    pub fn cleanup_unused_textures(&mut self) {
        if let Some(current_index) = self.selected_image_index {
            // Keep textures for current image and nearby images (±5)
            let keep_start = current_index.saturating_sub(5);
            let keep_end = (current_index + 6).min(self.images.len());
            
            for (i, image) in self.images.iter_mut().enumerate() {
                if i < keep_start || i >= keep_end {
                    // Clear full image texture for distant images
                    image.full_image = None;
                }
            }
        }
        
        // Clean up old thumbnails from cache
        self.ui_state.thumbnail_cache.cleanup_old_entries(100); // Keep last 100 accessed
    }
    
    /// Update frame rate tracking
    pub fn update_frame_timing(&mut self) {
        let now = Instant::now();
        if let Some(last_time) = self.ui_state.last_frame_time {
            let frame_time = now.duration_since(last_time);
            // If frame time is too high (low FPS), trigger cleanup
            if frame_time.as_millis() > 33 { // Less than 30 FPS
                self.cleanup_unused_textures();
            }
        }
        self.ui_state.last_frame_time = Some(now);
    }
    
    /// Handle file system errors with user-friendly messages
    pub fn handle_fs_error(&mut self, operation: &str, path: &Path, error: std::io::Error) {
        let user_message = match error.kind() {
            std::io::ErrorKind::NotFound => {
                format!("File or directory not found: {}", path.display())
            }
            std::io::ErrorKind::PermissionDenied => {
                format!("Permission denied accessing: {}", path.display())
            }
            std::io::ErrorKind::InvalidData => {
                format!("Invalid or corrupted file: {}", path.display())
            }
            _ => {
                format!("Failed to {} {}: {}", operation, path.display(), error)
            }
        };
        
        self.add_error_notification(user_message, ErrorType::Error, true);
    }
    
    /// Clean up old error notifications
    pub fn cleanup_notifications(&mut self) {
        let now = Instant::now();
        self.ui_state.error_notifications.retain(|notification| {
            if notification.auto_dismiss {
                now.duration_since(notification.timestamp).as_secs() < 10 // Auto-dismiss after 10 seconds
            } else {
                true
            }
        });
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
                    format!("Exposure value {:.2} is outside valid range [-3.0, 3.0] EV", value)
                );
            }
        }
        
        // Validate brightness values (-100 to +100)
        for (i, &value) in self.brightness.iter().enumerate() {
            if value < -100.0 || value > 100.0 {
                errors.insert(
                    format!("brightness[{}]", i),
                    format!("Brightness value {:.1} is outside valid range [-100, 100]", value)
                );
            }
        }
        
        // Validate contrast values (0.1 to 3.0)
        for (i, &value) in self.contrast.iter().enumerate() {
            if value < 0.1 || value > 3.0 {
                errors.insert(
                    format!("contrast[{}]", i),
                    format!("Contrast value {:.2}x is outside valid range [0.1, 3.0]", value)
                );
            }
        }
        
        // Validate saturation values (0.0 to 2.0)
        for (i, &value) in self.saturation.iter().enumerate() {
            if value < 0.0 || value > 2.0 {
                errors.insert(
                    format!("saturation[{}]", i),
                    format!("Saturation value {:.2}x is outside valid range [0.0, 2.0]", value)
                );
            }
        }
        
        // Validate offset values (reasonable range)
        for (i, &value) in self.offset_x.iter().enumerate() {
            if value < -5000.0 || value > 5000.0 {
                errors.insert(
                    format!("offset_x[{}]", i),
                    format!("X offset value {:.0}px is outside reasonable range [-5000, 5000]", value)
                );
            }
        }
        
        for (i, &value) in self.offset_y.iter().enumerate() {
            if value < -5000.0 || value > 5000.0 {
                errors.insert(
                    format!("offset_y[{}]", i),
                    format!("Y offset value {:.0}px is outside reasonable range [-5000, 5000]", value)
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
        
        // Validate threads (0 to 32)
        if self.threads > 32 {
            errors.insert(
                "threads".to_string(),
                format!("Thread count {} is outside reasonable range [0, 32]", self.threads)
            );
        }
        
        // Validate frame range
        if let (Some(start), Some(end)) = (self.start_frame, self.end_frame) {
            if start > end {
                errors.insert(
                    "frame_range".to_string(),
                    format!("Start frame ({}) must be less than or equal to end frame ({})", start, end)
                );
            }
            if start == 0 && end == 0 {
                errors.insert(
                    "frame_range".to_string(),
                    "Frame range cannot be 0-0. Use default values instead.".to_string()
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
        
        // Validate resolution format if provided
        if let Some(ref resolution) = self.resolution {
            if !resolution.is_empty() {
                let valid_presets = ["4K", "HD", "1080p", "720p"];
                let is_preset = valid_presets.iter().any(|&preset| 
                    resolution.to_lowercase() == preset.to_lowercase()
                );
                
                if !is_preset {
                    // Check if it's a valid WIDTHxHEIGHT format
                    let parts: Vec<&str> = resolution.split('x').collect();
                    if parts.len() != 2 {
                        errors.insert(
                            "resolution".to_string(),
                            format!("Resolution '{}' must be in format 'WIDTHxHEIGHT' (e.g., 1920x1080) or a preset (4K, HD, 1080p, 720p)", resolution)
                        );
                    } else {
                        // Validate width and height are numbers
                        if parts[0].parse::<u32>().is_err() || parts[1].parse::<u32>().is_err() {
                            errors.insert(
                                "resolution".to_string(),
                                format!("Resolution '{}' contains invalid numbers. Use format 'WIDTHxHEIGHT' (e.g., 1920x1080)", resolution)
                            );
                        } else {
                            let width: u32 = parts[0].parse().unwrap();
                            let height: u32 = parts[1].parse().unwrap();
                            if width < 64 || height < 64 {
                                errors.insert(
                                    "resolution".to_string(),
                                    format!("Resolution {}x{} is too small. Minimum is 64x64", width, height)
                                );
                            }
                            if width > 7680 || height > 4320 {
                                errors.insert(
                                    "resolution".to_string(),
                                    format!("Resolution {}x{} is too large. Maximum is 7680x4320 (8K)", width, height)
                                );
                            }
                        }
                    }
                }
            }
        }
        
        // Validate crop format if provided
        if let Some(ref crop_str) = self.crop {
            let parts: Vec<&str> = crop_str.split(':').collect();
            if parts.len() != 4 {
                errors.insert(
                    "crop".to_string(),
                    format!("Crop format '{}' is invalid. Must be 'width:height:x:y' (e.g., '1920:1080:100:50' or '80%:60%:10%:20%')", crop_str)
                );
            } else {
                // Validate each crop parameter
                for (i, part) in parts.iter().enumerate() {
                    let param_name = match i {
                        0 => "crop width",
                        1 => "crop height", 
                        2 => "crop x offset",
                        3 => "crop y offset",
                        _ => "crop parameter"
                    };
                    
                    if part.ends_with('%') {
                        // Percentage value
                        let percent_str = &part[..part.len()-1];
                        match percent_str.parse::<f32>() {
                            Ok(percent) => {
                                if percent < 0.0 || percent > 100.0 {
                                    errors.insert(
                                        format!("crop_{}", i),
                                        format!("{} percentage {:.1}% is outside valid range [0, 100]", param_name, percent)
                                    );
                                }
                            }
                            Err(_) => {
                                errors.insert(
                                    format!("crop_{}", i),
                                    format!("{} '{}' is not a valid percentage", param_name, part)
                                );
                            }
                        }
                    } else {
                        // Pixel value
                        match part.parse::<f32>() {
                            Ok(pixels) => {
                                if i < 2 && pixels <= 0.0 {
                                    errors.insert(
                                        format!("crop_{}", i),
                                        format!("{} {:.0}px must be greater than 0", param_name, pixels)
                                    );
                                }
                                if pixels.abs() > 10000.0 {
                                    errors.insert(
                                        format!("crop_{}", i),
                                        format!("{} {:.0}px is outside reasonable range [-10000, 10000]", param_name, pixels)
                                    );
                                }
                            }
                            Err(_) => {
                                errors.insert(
                                    format!("crop_{}", i),
                                    format!("{} '{}' is not a valid number", param_name, part)
                                );
                            }
                        }
                    }
                }
            }
        }
        
        // Parameter interdependency validation
        let _is_video_format = matches!(self.format.as_str(), "mp4" | "mov" | "avi");
        let is_image_format = matches!(self.format.as_str(), "jpg" | "png" | "tiff");
        
        if is_image_format {
            // For image formats, FPS and quality don't apply
            if self.fps != 24 {
                errors.insert(
                    "format_fps_conflict".to_string(),
                    format!("FPS setting is ignored for image format '{}'. Only applies to video formats.", self.format)
                );
            }
            if self.quality != 20 {
                errors.insert(
                    "format_quality_conflict".to_string(),
                    format!("Quality (CRF) setting is ignored for image format '{}'. Only applies to video formats.", self.format)
                );
            }
        }
        
        // Array length consistency warnings
        let array_lengths = [
            self.exposure.len(),
            self.brightness.len(), 
            self.contrast.len(),
            self.saturation.len(),
            self.offset_x.len(),
            self.offset_y.len()
        ];
        let max_array_len = array_lengths.iter().max().unwrap_or(&1);
        
        if *max_array_len > 1 {
            let arrays = [
                ("exposure", self.exposure.len()),
                ("brightness", self.brightness.len()),
                ("contrast", self.contrast.len()),
                ("saturation", self.saturation.len()),
                ("offset_x", self.offset_x.len()),
                ("offset_y", self.offset_y.len()),
            ];
            
            for (name, len) in arrays {
                if len > 1 && len != *max_array_len {
                    errors.insert(
                        format!("{}_array_length", name),
                        format!("{} array has {} values but max array length is {}. Consider matching lengths for consistent animation.", 
                            name, len, max_array_len)
                    );
                }
            }
        }
        
        errors
    }
    
    /// Generate CLI command arguments from settings
    pub fn generate_command_args(&self, input_dir: &Path, output_dir: &Path) -> Vec<String> {
        let mut args = Vec::new();
        
        // Input and output directories
        args.push("--input".to_string());
        args.push(input_dir.to_string_lossy().to_string());
        args.push("--output".to_string());
        args.push(output_dir.to_string_lossy().to_string());
        
        // Image adjustment parameters
        if self.exposure.len() == 1 && self.exposure[0] != 0.0 {
            args.push("--exposure".to_string());
            args.push(self.exposure[0].to_string());
        } else if self.exposure.len() > 1 {
            args.push("--exposure".to_string());
            args.push(self.exposure.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
        }
        
        if self.brightness.len() == 1 && self.brightness[0] != 0.0 {
            args.push("--brightness".to_string());
            args.push(self.brightness[0].to_string());
        } else if self.brightness.len() > 1 {
            args.push("--brightness".to_string());
            args.push(self.brightness.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
        }
        
        if self.contrast.len() == 1 && self.contrast[0] != 1.0 {
            args.push("--contrast".to_string());
            args.push(self.contrast[0].to_string());
        } else if self.contrast.len() > 1 {
            args.push("--contrast".to_string());
            args.push(self.contrast.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
        }
        
        if self.saturation.len() == 1 && self.saturation[0] != 1.0 {
            args.push("--saturation".to_string());
            args.push(self.saturation[0].to_string());
        } else if self.saturation.len() > 1 {
            args.push("--saturation".to_string());
            args.push(self.saturation.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
        }
        
        // Crop and positioning
        if let Some(ref crop) = self.crop {
            args.push("--crop".to_string());
            args.push(crop.clone());
        }
        
        if self.offset_x.len() == 1 && self.offset_x[0] != 0.0 {
            args.push("--offset-x".to_string());
            args.push(self.offset_x[0].to_string());
        } else if self.offset_x.len() > 1 {
            args.push("--offset-x".to_string());
            args.push(self.offset_x.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
        }
        
        if self.offset_y.len() == 1 && self.offset_y[0] != 0.0 {
            args.push("--offset-y".to_string());
            args.push(self.offset_y[0].to_string());
        } else if self.offset_y.len() > 1 {
            args.push("--offset-y".to_string());
            args.push(self.offset_y.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
        }
        
        // Output settings
        if self.format != "mp4" {
            args.push("--format".to_string());
            args.push(self.format.clone());
        }
        
        if self.fps != 24 {
            args.push("--fps".to_string());
            args.push(self.fps.to_string());
        }
        
        if self.quality != 20 {
            args.push("--quality".to_string());
            args.push(self.quality.to_string());
        }
        
        if let Some(ref resolution) = self.resolution {
            args.push("--resolution".to_string());
            args.push(resolution.clone());
        }
        
        // Processing settings
        if self.threads != 0 {
            args.push("--threads".to_string());
            args.push(self.threads.to_string());
        }
        
        if let Some(start_frame) = self.start_frame {
            args.push("--start-frame".to_string());
            args.push(start_frame.to_string());
        }
        
        if let Some(end_frame) = self.end_frame {
            args.push("--end-frame".to_string());
            args.push(end_frame.to_string());
        }
        
        args
    }
    
    /// Generate a preview of the CLI command
    pub fn generate_command_preview(&self, input_dir: &Path, output_dir: &Path) -> String {
        let args = self.generate_command_args(input_dir, output_dir);
        format!("lapsify {}", args.join(" "))
    }
}

/// Processing status for tracking time-lapse generation
#[derive(Default)]
pub struct ProcessingStatus {
    pub is_processing: bool,
    pub progress: f32,
    pub current_frame: usize,
    pub total_frames: usize,
    pub status_message: String,
    pub error_message: Option<String>,
    pub output_path: Option<PathBuf>,
    pub process_handle: Option<ProcessHandle>,
}

/// Handle for managing CLI process execution
pub struct ProcessHandle {
    pub process_id: u32,
    pub start_time: Instant,
    pub cancel_sender: mpsc::Sender<()>,
    pub progress_receiver: mpsc::Receiver<ProcessMessage>,
}

/// Messages from CLI process
#[derive(Debug, Clone)]
pub enum ProcessMessage {
    Progress { current: usize, total: usize, message: String },
    Output(String),
    Error(String),
    Finished { success: bool, output_path: Option<PathBuf> },
}

/// Commands to CLI process
#[derive(Debug, Clone)]
pub enum ProcessCommand {
    Cancel,
}

/// CLI execution result
#[derive(Debug, Clone)]
pub struct CliResult {
    pub success: bool,
    pub output_path: Option<PathBuf>,
    pub error_message: Option<String>,
    pub stdout: String,
    pub stderr: String,
}

/// Thumbnail cache entry with metadata
#[derive(Clone)]
pub struct ThumbnailCacheEntry {
    pub texture: egui::TextureHandle,
    pub last_accessed: Instant,
    pub memory_size: usize,
}

/// LRU cache for thumbnails with memory management
#[derive(Default)]
pub struct ThumbnailCache {
    pub entries: HashMap<PathBuf, ThumbnailCacheEntry>,
    pub access_order: VecDeque<PathBuf>,
    pub max_entries: usize,
    pub max_memory_mb: usize,
    pub current_memory_bytes: usize,
}

impl ThumbnailCache {
    pub fn new(max_entries: usize, max_memory_mb: usize) -> Self {
        Self {
            entries: HashMap::new(),
            access_order: VecDeque::new(),
            max_entries,
            max_memory_mb,
            current_memory_bytes: 0,
        }
    }
    
    pub fn get(&mut self, path: &PathBuf) -> Option<egui::TextureHandle> {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.last_accessed = Instant::now();
            // Move to front of access order
            if let Some(pos) = self.access_order.iter().position(|p| p == path) {
                self.access_order.remove(pos);
            }
            self.access_order.push_front(path.clone());
            Some(entry.texture.clone())
        } else {
            None
        }
    }
    
    pub fn insert(&mut self, path: PathBuf, texture: egui::TextureHandle, memory_size: usize) {
        // Remove existing entry if present
        if let Some(old_entry) = self.entries.remove(&path) {
            self.current_memory_bytes -= old_entry.memory_size;
            if let Some(pos) = self.access_order.iter().position(|p| p == &path) {
                self.access_order.remove(pos);
            }
        }
        
        // Add new entry
        let entry = ThumbnailCacheEntry {
            texture,
            last_accessed: Instant::now(),
            memory_size,
        };
        
        self.entries.insert(path.clone(), entry);
        self.access_order.push_front(path);
        self.current_memory_bytes += memory_size;
        
        // Enforce cache limits
        self.enforce_limits();
    }
    
    fn enforce_limits(&mut self) {
        let max_memory_bytes = self.max_memory_mb * 1024 * 1024;
        
        // Remove entries until we're under both limits
        while (self.entries.len() > self.max_entries || self.current_memory_bytes > max_memory_bytes)
            && !self.access_order.is_empty() {
            
            if let Some(oldest_path) = self.access_order.pop_back() {
                if let Some(entry) = self.entries.remove(&oldest_path) {
                    self.current_memory_bytes -= entry.memory_size;
                }
            }
        }
    }
    
    pub fn clear(&mut self) {
        self.entries.clear();
        self.access_order.clear();
        self.current_memory_bytes = 0;
    }
    
    pub fn memory_usage_mb(&self) -> f32 {
        self.current_memory_bytes as f32 / (1024.0 * 1024.0)
    }
    
    /// Check if a thumbnail is cached
    pub fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }
    
    /// Clean up old entries, keeping only the most recently accessed ones
    pub fn cleanup_old_entries(&mut self, keep_count: usize) {
        if self.entries.len() <= keep_count {
            return;
        }
        
        // Sort by last accessed time and keep only the most recent
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by(|a, b| b.1.last_accessed.cmp(&a.1.last_accessed));
        
        // Remove old entries
        let to_remove: Vec<_> = entries.iter().skip(keep_count).map(|(path, _)| (*path).clone()).collect();
        
        for path in to_remove {
            if let Some(entry) = self.entries.remove(&path) {
                self.current_memory_bytes = self.current_memory_bytes.saturating_sub(entry.memory_size);
            }
        }
    }
}

/// Thumbnail loading state
#[derive(Clone, PartialEq)]
pub enum ThumbnailLoadState {
    NotStarted,
    Loading,
    Loaded,
    Error(String),
}

/// Error notification for non-blocking error display
#[derive(Clone, Debug)]
pub struct ErrorNotification {
    pub message: String,
    pub error_type: ErrorType,
    pub timestamp: Instant,
    pub auto_dismiss: bool,
}

/// Types of errors for different handling
#[derive(Clone, Debug, PartialEq)]
pub enum ErrorType {
    Info,
    Warning,
    Error,
    Critical,
}

/// Modal dialog state for critical errors
#[derive(Default)]
pub struct ModalDialog {
    pub is_open: bool,
    pub title: String,
    pub message: String,
    pub dialog_type: DialogType,
}

/// Types of modal dialogs
#[derive(Default, PartialEq, Clone)]
pub enum DialogType {
    #[default]
    Error,
    Confirmation,
    Info,
}

/// UI state for managing interface elements
#[derive(Serialize, Deserialize)]
pub struct UiState {
    pub sidebar_width: f32,
    pub carousel_height: f32,
    pub show_settings_validation: bool,
    #[serde(skip)]
    pub validation_errors: HashMap<String, String>,
    pub zoom_level: f32,
    #[serde(skip)]
    pub pan_offset: egui::Vec2,
    #[serde(skip)]
    pub folder_error: Option<String>,
    #[serde(skip)]
    pub thumbnail_cache: ThumbnailCache,
    #[serde(skip)]
    pub thumbnail_load_states: HashMap<PathBuf, ThumbnailLoadState>,
    #[serde(skip)]
    pub carousel_scroll_offset: f32,
    #[serde(skip)]
    pub visible_thumbnail_range: (usize, usize),
    pub output_directory: Option<PathBuf>,
    pub window_size: Option<(f32, f32)>,
    pub window_position: Option<(f32, f32)>,
    #[serde(skip)]
    pub error_notifications: VecDeque<ErrorNotification>,
    #[serde(skip)]
    pub modal_dialog: ModalDialog,
    #[serde(skip)]
    pub lapsify_cli_available: Option<bool>,
    #[serde(skip)]
    pub show_help_dialog: bool,
    #[serde(skip)]
    pub background_load_queue: VecDeque<PathBuf>,
    #[serde(skip)]
    pub last_frame_time: Option<Instant>,
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
            thumbnail_cache: ThumbnailCache::new(100, 50), // 100 thumbnails, 50MB max
            thumbnail_load_states: HashMap::new(),
            carousel_scroll_offset: 0.0,
            visible_thumbnail_range: (0, 0),
            output_directory: None,
            window_size: None,
            window_position: None,
            error_notifications: VecDeque::new(),
            modal_dialog: ModalDialog::default(),
            lapsify_cli_available: None,
            show_help_dialog: false,
            background_load_queue: VecDeque::new(),
            last_frame_time: None,
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

/// Generate a thumbnail from an image with size constraints
fn generate_thumbnail(img: &DynamicImage, max_size: u32) -> DynamicImage {
    let (width, height) = img.dimensions();
    
    // Calculate thumbnail dimensions maintaining aspect ratio
    let (thumb_width, thumb_height) = if width > height {
        let ratio = max_size as f32 / width as f32;
        (max_size, (height as f32 * ratio) as u32)
    } else {
        let ratio = max_size as f32 / height as f32;
        ((width as f32 * ratio) as u32, max_size)
    };
    
    // Resize using high-quality filtering
    img.resize(thumb_width, thumb_height, FilterType::Lanczos3)
}

/// Convert DynamicImage to egui ColorImage
fn dynamic_image_to_color_image(img: &DynamicImage) -> egui::ColorImage {
    let rgba_img = img.to_rgba8();
    let (width, height) = rgba_img.dimensions();
    let pixels = rgba_img.into_raw();
    
    egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        &pixels,
    )
}

/// Calculate approximate memory usage of a thumbnail
fn calculate_thumbnail_memory_size(width: u32, height: u32) -> usize {
    // RGBA = 4 bytes per pixel
    (width * height * 4) as usize
}

// Carousel constants
const THUMBNAIL_SIZE: f32 = 120.0;
const THUMBNAIL_SPACING: f32 = 8.0;
const CAROUSEL_PADDING: f32 = 10.0;

// Image viewer constants
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 10.0;
const ZOOM_SPEED: f32 = 0.1;

// Settings constants
const ARRAY_INPUT_WIDTH: f32 = 200.0;

/// Load thumbnail asynchronously
fn load_thumbnail_async(path: &PathBuf) -> Result<(egui::ColorImage, usize), String> {
    // Load the image
    let img = image::open(path)
        .map_err(|e| format!("Failed to open image: {}", e))?;
    
    // Generate thumbnail with 200x200 max size
    let thumbnail = generate_thumbnail(&img, 200);
    
    // Convert to egui ColorImage
    let color_image = dynamic_image_to_color_image(&thumbnail);
    
    // Calculate memory usage
    let memory_size = calculate_thumbnail_memory_size(
        color_image.width() as u32,
        color_image.height() as u32
    );
    
    Ok((color_image, memory_size))
}

/// Load full-size image for main viewer
fn load_full_image_async(path: &PathBuf) -> Result<egui::ColorImage, String> {
    // Load the image
    let img = image::open(path)
        .map_err(|e| format!("Failed to open image: {}", e))?;
    
    // For very large images, we might want to limit the size to prevent memory issues
    let (width, height) = img.dimensions();
    let max_dimension = 2048; // Limit to 2048px on longest side
    
    let processed_img = if width > max_dimension || height > max_dimension {
        // Resize large images to prevent memory issues
        let scale = max_dimension as f32 / width.max(height) as f32;
        let new_width = (width as f32 * scale) as u32;
        let new_height = (height as f32 * scale) as u32;
        img.resize(new_width, new_height, FilterType::Lanczos3)
    } else {
        img
    };
    
    // Convert to egui ColorImage
    Ok(dynamic_image_to_color_image(&processed_img))
}

/// Execute lapsify CLI command with progress monitoring
fn execute_lapsify_command_with_progress(
    args: Vec<String>, 
    output_dir: PathBuf, 
    total_frames: usize,
    progress_sender: mpsc::Sender<ProcessMessage>,
    cancel_receiver: mpsc::Receiver<()>
) -> Result<CliResult, String> {
    // Try to find lapsify executable
    let lapsify_cmd = find_lapsify_executable()?;
    
    println!("Executing: {} {}", lapsify_cmd, args.join(" "));
    
    // Send initial progress
    let _ = progress_sender.send(ProcessMessage::Progress {
        current: 0,
        total: total_frames,
        message: "Starting lapsify CLI...".to_string(),
    });
    
    // Execute the command with streaming output
    let mut command = Command::new(&lapsify_cmd);
    command.args(&args);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    
    let mut child = command.spawn()
        .map_err(|e| format!("Failed to spawn lapsify command: {}", e))?;
    
    // Monitor process output and cancellation
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    
    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);
    
    let progress_sender_clone = progress_sender.clone();
    let cancel_receiver_clone = Arc::new(Mutex::new(cancel_receiver));
    
    // Monitor stdout for progress information
    let stdout_handle = {
        let progress_sender = progress_sender_clone.clone();
        thread::spawn(move || {
            for line in stdout_reader.lines() {
                if let Ok(line) = line {
                    // Parse progress from CLI output
                    if let Some((current, total)) = parse_progress_from_output(&line) {
                        let _ = progress_sender.send(ProcessMessage::Progress {
                            current,
                            total,
                            message: format!("Processing frame {} of {}", current, total),
                        });
                    } else {
                        let _ = progress_sender.send(ProcessMessage::Output(line));
                    }
                }
            }
        })
    };
    
    // Monitor stderr for errors
    let stderr_handle = {
        let progress_sender = progress_sender_clone.clone();
        thread::spawn(move || {
            for line in stderr_reader.lines() {
                if let Ok(line) = line {
                    let _ = progress_sender.send(ProcessMessage::Error(line));
                }
            }
        })
    };
    
    // Monitor for cancellation
    let _cancel_handle = {
        let cancel_receiver = cancel_receiver_clone.clone();
        thread::spawn(move || {
            if let Ok(cancel_receiver) = cancel_receiver.lock() {
                if cancel_receiver.recv().is_ok() {
                    // Process cancellation requested
                    println!("Process cancellation requested");
                }
            }
        })
    };
    
    // Wait for process completion
    let output = child.wait_with_output()
        .map_err(|e| format!("Failed to wait for lapsify command: {}", e))?;
    
    // Clean up monitoring threads
    let _ = stdout_handle.join();
    let _ = stderr_handle.join();
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    
    let success = output.status.success();
    
    // Try to determine output file path
    let output_path = if success {
        find_output_file(&output_dir, &args)
    } else {
        None
    };
    
    let error_message = if !success {
        Some(if stderr.is_empty() { 
            "Command failed with unknown error".to_string() 
        } else { 
            stderr.clone() 
        })
    } else {
        None
    };
    
    // Send final progress message
    let _ = progress_sender.send(ProcessMessage::Finished {
        success,
        output_path: output_path.clone(),
    });
    
    Ok(CliResult {
        success,
        output_path,
        error_message,
        stdout,
        stderr,
    })
}

/// Execute lapsify CLI command (simple version for compatibility)
fn execute_lapsify_command(args: Vec<String>, output_dir: PathBuf) -> Result<CliResult, String> {
    let (progress_sender, _) = mpsc::channel();
    let (_, cancel_receiver) = mpsc::channel();
    execute_lapsify_command_with_progress(args, output_dir, 0, progress_sender, cancel_receiver)
}

/// Parse progress information from CLI output
fn parse_progress_from_output(line: &str) -> Option<(usize, usize)> {
    // Look for patterns like "Processing 5/100" or "Frame 5 of 100"
    // Simple string parsing to avoid regex complexity
    let line_lower = line.to_lowercase();
    
    if line_lower.contains("processing") || line_lower.contains("frame") {
        // Extract numbers from the line
        let numbers: Vec<usize> = line
            .split_whitespace()
            .filter_map(|word| {
                // Try to parse numbers, including those with separators like "5/100"
                if word.contains('/') {
                    let parts: Vec<&str> = word.split('/').collect();
                    if parts.len() == 2 {
                        if let (Ok(current), Ok(_total)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
                            return Some(current); // Return current, we'll handle total separately
                        }
                    }
                }
                word.parse().ok()
            })
            .collect();
        
        if numbers.len() >= 2 {
            Some((numbers[0], numbers[1]))
        } else if numbers.len() == 1 {
            Some((numbers[0], 0))
        } else {
            None
        }
    } else {
        None
    }
}

/// Find lapsify executable in system PATH or current directory
fn find_lapsify_executable() -> Result<String, String> {
    // Try different possible locations
    let candidates = [
        "lapsify",           // In PATH
        "./lapsify",         // Current directory
        "./target/debug/lapsify",    // Debug build
        "./target/release/lapsify",  // Release build
        "cargo run --bin lapsify --", // Fallback to cargo
    ];
    
    for candidate in &candidates {
        if candidate.starts_with("cargo") {
            // Special case for cargo run
            return Ok(candidate.to_string());
        }
        
        // Test if executable exists and is runnable
        if let Ok(output) = Command::new(candidate)
            .arg("--help")
            .output() 
        {
            if output.status.success() {
                return Ok(candidate.to_string());
            }
        }
    }
    
    Err("Could not find lapsify executable. Please ensure lapsify is installed or built.".to_string())
}

/// Find output file in the output directory
fn find_output_file(output_dir: &Path, args: &[String]) -> Option<PathBuf> {
    // Extract format from args
    let format = args.iter()
        .position(|arg| arg == "--format")
        .and_then(|i| args.get(i + 1))
        .unwrap_or(&"mp4".to_string())
        .clone();
    
    // Look for files with the expected extension
    let extensions = match format.as_str() {
        "mp4" => vec!["mp4"],
        "mov" => vec!["mov"],
        "avi" => vec!["avi"],
        "jpg" => vec!["jpg", "jpeg"],
        "png" => vec!["png"],
        "tiff" => vec!["tiff", "tif"],
        _ => vec!["mp4"], // Default fallback
    };
    
    // Look for recently created files with matching extensions
    if let Ok(entries) = fs::read_dir(output_dir) {
        let mut candidates: Vec<_> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                if let Some(ext) = entry.path().extension() {
                    if let Some(ext_str) = ext.to_str() {
                        return extensions.contains(&ext_str.to_lowercase().as_str());
                    }
                }
                false
            })
            .collect();
        
        // Sort by modification time (most recent first)
        candidates.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            let b_time = b.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            b_time.cmp(&a_time)
        });
        
        // Return the most recently created file
        candidates.first().map(|entry| entry.path())
    } else {
        None
    }
}

/// Get session directory for storing app data
fn get_session_dir() -> Result<PathBuf, String> {
    let home_dir = dirs::home_dir()
        .ok_or("Could not find home directory")?;
    
    Ok(home_dir.join(".lapsify-gui"))
}

/// Create default settings presets
fn create_default_presets() -> Vec<SettingsPreset> {
    vec![
        SettingsPreset {
            name: "Default".to_string(),
            description: "Standard time-lapse settings".to_string(),
            settings: LapsifySettings::default(),
        },
        SettingsPreset {
            name: "High Quality".to_string(),
            description: "High quality video output with enhanced contrast".to_string(),
            settings: LapsifySettings {
                contrast: vec![1.2],
                saturation: vec![1.1],
                quality: 18,
                fps: 30,
                ..Default::default()
            },
        },
        SettingsPreset {
            name: "Fast Preview".to_string(),
            description: "Quick preview with lower quality".to_string(),
            settings: LapsifySettings {
                quality: 28,
                fps: 15,
                resolution: Some("720p".to_string()),
                ..Default::default()
            },
        },
        SettingsPreset {
            name: "Sunset Enhancement".to_string(),
            description: "Enhanced colors for sunset/sunrise time-lapses".to_string(),
            settings: LapsifySettings {
                exposure: vec![0.3],
                brightness: vec![5.0],
                contrast: vec![1.3],
                saturation: vec![1.4],
                ..Default::default()
            },
        },
        SettingsPreset {
            name: "Night Sky".to_string(),
            description: "Settings optimized for night sky time-lapses".to_string(),
            settings: LapsifySettings {
                exposure: vec![0.8],
                brightness: vec![10.0],
                contrast: vec![1.5],
                saturation: vec![0.9],
                ..Default::default()
            },
        },
    ]
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
                            self.state.add_error_notification(
                                format!("Error scanning images: {}", error),
                                ErrorType::Error,
                                true,
                            );
                        }
                    }
                }
                Err(error) => {
                    // Store the validation error for display
                    self.state.ui_state.folder_error = Some(error.clone());
                    self.state.add_error_notification(
                        error,
                        ErrorType::Warning,
                        true,
                    );
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
    
    /// Load thumbnails for visible/priority images
    fn load_visible_thumbnails(&mut self, ctx: &egui::Context) {
        // Load thumbnail for currently selected image first
        if let Some(selected_index) = self.state.selected_image_index {
            self.state.load_thumbnail_sync(selected_index, ctx);
        }
        
        // Load thumbnails for first few images (for carousel display)
        let visible_count = std::cmp::min(10, self.state.images.len());
        for i in 0..visible_count {
            self.state.load_thumbnail_sync(i, ctx);
        }
    }
    
    /// Load thumbnails for images visible in the carousel viewport (optimized)
    fn load_visible_carousel_thumbnails(&mut self, ctx: &egui::Context) {
        let (start, end) = self.state.ui_state.visible_thumbnail_range;
        
        // Load thumbnails for visible range plus a small buffer
        let buffer = 2; // Reduced buffer for better performance
        let start_with_buffer = start.saturating_sub(buffer);
        let end_with_buffer = std::cmp::min(end + buffer, self.state.images.len());
        
        // Prioritize loading thumbnails in the visible range first
        for i in start..end {
            if i < self.state.images.len() {
                let image_path = &self.state.images[i].path;
                if !self.state.ui_state.thumbnail_cache.contains(image_path) {
                    self.state.load_thumbnail_sync(i, ctx);
                    // Only load one thumbnail per frame to maintain smooth UI
                    return;
                }
            }
        }
        
        // Then load buffer thumbnails if visible ones are already loaded
        for i in start_with_buffer..start {
            if i < self.state.images.len() {
                let image_path = &self.state.images[i].path;
                if !self.state.ui_state.thumbnail_cache.contains(image_path) {
                    self.state.load_thumbnail_sync(i, ctx);
                    return;
                }
            }
        }
        
        for i in end..end_with_buffer {
            if i < self.state.images.len() {
                let image_path = &self.state.images[i].path;
                if !self.state.ui_state.thumbnail_cache.contains(image_path) {
                    self.state.load_thumbnail_sync(i, ctx);
                    return;
                }
            }
        }
    }
    
    /// Calculate which thumbnails are visible in the carousel viewport
    fn calculate_visible_thumbnails(&mut self, scroll_area_rect: egui::Rect, scroll_offset: f32) {
        let thumbnail_width = THUMBNAIL_SIZE + THUMBNAIL_SPACING;
        let viewport_start = scroll_offset;
        let viewport_end = scroll_offset + scroll_area_rect.width();
        
        let start_index = ((viewport_start - CAROUSEL_PADDING) / thumbnail_width).floor().max(0.0) as usize;
        let end_index = ((viewport_end - CAROUSEL_PADDING) / thumbnail_width).ceil() as usize;
        
        let end_index = std::cmp::min(end_index, self.state.images.len());
        
        self.state.ui_state.visible_thumbnail_range = (start_index, end_index);
    }
    
    /// Select output directory for processed results
    fn select_output_directory(&mut self) {
        if let Some(output_dir) = rfd::FileDialog::new()
            .set_title("Select Output Directory")
            .pick_folder()
        {
            self.state.ui_state.output_directory = Some(output_dir);
        }
    }
    
    /// Execute lapsify CLI with current settings
    fn execute_lapsify_cli(&mut self, ctx: &egui::Context) -> Result<(), String> {
        // Validate prerequisites
        let input_dir = self.state.selected_folder.as_ref()
            .ok_or("No input folder selected")?;
        
        let output_dir = self.state.ui_state.output_directory.as_ref()
            .ok_or("No output directory selected")?;
        
        if self.state.images.is_empty() {
            return Err("No images found in input folder".to_string());
        }
        
        // Validate settings
        let validation_errors = self.state.settings.validate();
        if !validation_errors.is_empty() {
            let error_count = validation_errors.len();
            return Err(format!("Settings validation failed with {} errors. Please fix validation errors before processing.", error_count));
        }
        
        // Generate command arguments
        let args = self.state.settings.generate_command_args(input_dir, output_dir);
        
        // Set up communication channels
        let (progress_sender, progress_receiver) = mpsc::channel();
        let (cancel_sender, cancel_receiver) = mpsc::channel();
        
        // Set up processing status
        self.state.processing_status.is_processing = true;
        self.state.processing_status.progress = 0.0;
        self.state.processing_status.current_frame = 0;
        self.state.processing_status.total_frames = self.state.images.len();
        self.state.processing_status.status_message = "Starting lapsify CLI...".to_string();
        self.state.processing_status.error_message = None;
        self.state.processing_status.output_path = None;
        self.state.processing_status.process_handle = Some(ProcessHandle {
            process_id: 0, // Will be set when process starts
            start_time: Instant::now(),
            cancel_sender,
            progress_receiver,
        });
        
        // Execute CLI in background thread with progress monitoring
        let ctx_clone = ctx.clone();
        let args_clone = args.clone();
        let output_dir_clone = output_dir.clone();
        let total_frames = self.state.images.len();
        
        thread::spawn(move || {
            let progress_sender_clone = progress_sender.clone();
            match execute_lapsify_command_with_progress(args_clone, output_dir_clone, total_frames, progress_sender, cancel_receiver) {
                Ok(result) => {
                    println!("CLI execution completed: {:?}", result);
                    ctx_clone.request_repaint();
                }
                Err(error) => {
                    println!("CLI execution failed: {}", error);
                    // Send error through progress channel
                    let _ = progress_sender_clone.send(ProcessMessage::Error(error));
                    ctx_clone.request_repaint();
                }
            }
        });
        
        Ok(())
    }
    
    /// Cancel current CLI execution
    fn cancel_cli_execution(&mut self) {
        if self.state.processing_status.is_processing {
            if let Some(handle) = &self.state.processing_status.process_handle {
                // Send cancel signal to background thread
                let _ = handle.cancel_sender.send(());
            }
            
            self.state.processing_status.is_processing = false;
            self.state.processing_status.status_message = "Cancelling processing...".to_string();
            self.state.processing_status.process_handle = None;
        }
    }
    
    /// Update processing status from background thread
    fn update_processing_status(&mut self) {
        let mut messages_to_process = Vec::new();
        let mut should_clear_handle = false;
        
        // Collect messages without holding a borrow
        if let Some(handle) = &self.state.processing_status.process_handle {
            while let Ok(message) = handle.progress_receiver.try_recv() {
                messages_to_process.push(message);
            }
        }
        
        // Process collected messages
        for message in messages_to_process {
            match message {
                ProcessMessage::Progress { current, total, message } => {
                    self.state.processing_status.current_frame = current;
                    self.state.processing_status.total_frames = total;
                    self.state.processing_status.progress = if total > 0 {
                        current as f32 / total as f32
                    } else {
                        0.0
                    };
                    self.state.processing_status.status_message = message;
                }
                ProcessMessage::Output(output) => {
                    // Update status with CLI output
                    self.state.processing_status.status_message = format!("Processing: {}", output);
                }
                ProcessMessage::Error(error) => {
                    self.state.processing_status.error_message = Some(error.clone());
                    self.state.processing_status.is_processing = false;
                    should_clear_handle = true;
                    self.state.add_error_notification(
                        format!("Processing error: {}", error),
                        ErrorType::Error,
                        false,
                    );
                }
                ProcessMessage::Finished { success, output_path } => {
                    self.state.processing_status.is_processing = false;
                    should_clear_handle = true;
                    
                    if success {
                        self.state.processing_status.status_message = "Processing completed successfully!".to_string();
                        self.state.processing_status.output_path = output_path;
                        self.state.processing_status.progress = 1.0;
                    } else {
                        self.state.processing_status.status_message = "Processing failed".to_string();
                        if self.state.processing_status.error_message.is_none() {
                            self.state.processing_status.error_message = Some("Unknown error occurred".to_string());
                        }
                    }
                }
            }
        }
        
        // Clear handle if needed
        if should_clear_handle {
            self.state.processing_status.process_handle = None;
        }
    }
    
    /// Handle zoom input
    fn handle_zoom(&mut self, delta: f32) {
        let new_zoom = (self.state.ui_state.zoom_level + delta * ZOOM_SPEED).clamp(MIN_ZOOM, MAX_ZOOM);
        self.state.ui_state.zoom_level = new_zoom;
    }
    
    /// Reset zoom and pan to default
    fn reset_view(&mut self) {
        self.state.ui_state.zoom_level = 1.0;
        self.state.ui_state.pan_offset = egui::Vec2::ZERO;
    }
    
        /// Calculate image display size maintaining aspect ratio
    fn calculate_image_display_size(&self, image_size: egui::Vec2, available_size: egui::Vec2) -> egui::Vec2 {
        let scale_x = available_size.x / image_size.x;
        let scale_y = available_size.y / image_size.y;
        let scale = scale_x.min(scale_y) * self.state.ui_state.zoom_level;
        
        egui::Vec2::new(image_size.x * scale, image_size.y * scale)
    }
    
    /// Save current settings to file
    fn save_settings_to_file(&self) -> Result<(), String> {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Save Settings")
            .add_filter("JSON files", &["json"])
            .set_file_name("lapsify_settings.json")
            .save_file()
        {
            self.state.settings.save_to_file(&path)
                .map_err(|e| format!("Failed to save settings: {}", e))?;
        }
        Ok(())
    }
    
    /// Load settings from file
    fn load_settings_from_file(&mut self) -> Result<(), String> {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Load Settings")
            .add_filter("JSON files", &["json"])
            .pick_file()
        {
            self.state.settings = LapsifySettings::load_from_file(&path)
                .map_err(|e| format!("Failed to load settings: {}", e))?;
            self.state.validate_settings();
        }
        Ok(())
    }
    
    /// Apply settings preset
    fn apply_preset(&mut self, preset_index: usize) {
        if let Some(preset) = self.state.settings_presets.get(preset_index) {
            self.state.settings = preset.settings.clone();
            self.state.validate_settings();
        }
    }
    
    /// Save current settings as new preset
    fn save_as_preset(&mut self, name: String, description: String) {
        let preset = SettingsPreset {
            name,
            description,
            settings: self.state.settings.clone(),
        };
        
        self.state.settings_presets.push(preset);
        let _ = self.state.save_presets();
    }
    
    /// Update window state for persistence
    fn update_window_state(&mut self, ctx: &egui::Context) {
        let viewport = ctx.input(|i| i.viewport().clone());
        if let Some(inner_rect) = viewport.inner_rect {
            self.state.ui_state.window_size = Some((inner_rect.width(), inner_rect.height()));
            self.state.ui_state.window_position = Some((inner_rect.min.x, inner_rect.min.y));
        }
    }
    
    /// Apply responsive layout adjustments
    fn apply_responsive_layout(&mut self, available_size: egui::Vec2) {
        // Adjust sidebar width based on screen size
        let min_sidebar_width = 250.0;
        let max_sidebar_width = 400.0;
        let optimal_sidebar_ratio = 0.25; // 25% of screen width
        
        let optimal_width = available_size.x * optimal_sidebar_ratio;
        self.state.ui_state.sidebar_width = optimal_width.clamp(min_sidebar_width, max_sidebar_width);
        
        // Adjust carousel height based on screen size
        let min_carousel_height = 100.0;
        let max_carousel_height = 250.0;
        let optimal_carousel_ratio = 0.2; // 20% of screen height
        
        let optimal_height = available_size.y * optimal_carousel_ratio;
        self.state.ui_state.carousel_height = optimal_height.clamp(min_carousel_height, max_carousel_height);
    }
    
    /// Show array input widget for animation parameters with validation
    fn show_array_input(ui: &mut egui::Ui, label: &str, values: &mut Vec<f32>, min: f32, max: f32, unit: &str, validation_errors: &HashMap<String, String>) -> bool {
        let mut changed = false;
        
        // Check if this parameter has validation errors
        let param_key = label.to_lowercase().replace(" ", "_");
        let has_errors = validation_errors.keys().any(|k| k.starts_with(&param_key));
        
        ui.horizontal(|ui| {
            // Show error indicator if there are validation errors
            if has_errors {
                ui.colored_label(ui.visuals().error_fg_color, "⚠");
            }
            
            ui.label(format!("{}:", label));
            ui.add_space(5.0);
            
            // Show current values as comma-separated string
            let values_str = values.iter()
                .map(|v| format!("{:.2}", v))
                .collect::<Vec<_>>()
                .join(", ");
            
            let text_color = if has_errors {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            
            ui.colored_label(text_color, format!("[{}] {}", values_str, unit));
        });
        
        // Show validation errors for this parameter
        for (error_key, error_msg) in validation_errors {
            if error_key.starts_with(&param_key) {
                ui.indent("error_indent", |ui| {
                    ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error_msg));
                });
            }
        }
        
        // Individual value controls
        ui.indent("array_controls", |ui| {
            let mut to_remove = None;
            let values_len = values.len();
            
            for (i, value) in values.iter_mut().enumerate() {
                // Check if this specific array element has an error
                let element_key = format!("{}[{}]", param_key, i);
                let element_has_error = validation_errors.contains_key(&element_key);
                
                ui.horizontal(|ui| {
                    if element_has_error {
                        ui.colored_label(ui.visuals().error_fg_color, "⚠");
                    }
                    
                    ui.label(format!("{}:", i + 1));
                    
                    let mut slider = egui::Slider::new(value, min..=max)
                        .step_by(0.01)
                        .fixed_decimals(2);
                    
                    // Color the slider differently if there's an error
                    if element_has_error {
                        slider = slider.text_color(ui.visuals().error_fg_color);
                    }
                    
                    let response = ui.add(slider);
                    
                    if response.changed() {
                        changed = true;
                    }
                    
                    ui.label(unit);
                    
                    // Remove button (only if more than one value)
                    if values_len > 1 && ui.small_button("❌").clicked() {
                        to_remove = Some(i);
                        changed = true;
                    }
                });
            }
            
            // Remove value if requested
            if let Some(index) = to_remove {
                values.remove(index);
            }
            
            // Add value button
            ui.horizontal(|ui| {
                if ui.button("+ Add Value").clicked() {
                    values.push(values.last().copied().unwrap_or(0.0));
                    changed = true;
                }
                
                // Reset to single value button
                if values.len() > 1 && ui.button("Reset to Single").clicked() {
                    let first_value = values[0];
                    values.clear();
                    values.push(first_value);
                    changed = true;
                }
            });
        });
        
        changed
    }
    
    /// Show crop parameter input with validation
    fn show_crop_input(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let validation_errors = &self.state.ui_state.validation_errors;
        
        // Check for crop-related errors
        let has_crop_errors = validation_errors.keys().any(|k| k.starts_with("crop"));
        
        ui.horizontal(|ui| {
            if has_crop_errors {
                ui.colored_label(ui.visuals().error_fg_color, "⚠");
            }
            ui.label("Crop:");
            
            let crop_enabled = self.state.settings.crop.is_some();
            let mut enable_crop = crop_enabled;
            
            if ui.checkbox(&mut enable_crop, "Enable").changed() {
                if enable_crop && !crop_enabled {
                    // Enable crop with default values
                    self.state.settings.crop = Some("50%:50%:25%:25%".to_string());
                    changed = true;
                } else if !enable_crop && crop_enabled {
                    // Disable crop
                    self.state.settings.crop = None;
                    changed = true;
                }
            }
        });
        
        // Show crop validation errors
        for (key, error) in validation_errors {
            if key == "crop" {
                ui.indent("crop_error", |ui| {
                    ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                });
            }
        }
        
        if let Some(crop_str) = &mut self.state.settings.crop {
            ui.indent("crop_controls", |ui| {
                // Parse current crop values
                let parts: Vec<&str> = crop_str.split(':').collect();
                if parts.len() == 4 {
                    let mut width_str = parts[0].to_string();
                    let mut height_str = parts[1].to_string();
                    let mut x_str = parts[2].to_string();
                    let mut y_str = parts[3].to_string();
                    
                    ui.horizontal(|ui| {
                        // Check for individual crop parameter errors
                        if validation_errors.contains_key("crop_0") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Width:");
                        if ui.text_edit_singleline(&mut width_str).changed() {
                            changed = true;
                        }
                        
                        if validation_errors.contains_key("crop_1") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Height:");
                        if ui.text_edit_singleline(&mut height_str).changed() {
                            changed = true;
                        }
                    });
                    
                    ui.horizontal(|ui| {
                        if validation_errors.contains_key("crop_2") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("X offset:");
                        if ui.text_edit_singleline(&mut x_str).changed() {
                            changed = true;
                        }
                        
                        if validation_errors.contains_key("crop_3") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Y offset:");
                        if ui.text_edit_singleline(&mut y_str).changed() {
                            changed = true;
                        }
                    });
                    
                    // Show individual crop parameter errors
                    for i in 0..4 {
                        if let Some(error) = validation_errors.get(&format!("crop_{}", i)) {
                            ui.indent("crop_param_error", |ui| {
                                ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                            });
                        }
                    }
                    
                    if changed {
                        *crop_str = format!("{}:{}:{}:{}", width_str, height_str, x_str, y_str);
                    }
                    
                    ui.label("Format: width:height:x:y (use % for percentages)");
                    ui.label("Example: 1920:1080:100:50 or 80%:60%:10%:20%");
                }
            });
        }
        
        changed
    }

    /// Display the settings sidebar panel
    fn show_settings_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.separator();
        
        // Folder selection section
        ui.heading("Input Folder");
        
        // Folder selection and refresh buttons
        ui.horizontal(|ui| {
            if ui.button("📁 Select Folder")
                .on_hover_text("Select a folder containing images (Ctrl+O)")
                .clicked() {
                self.select_folder();
            }
            
            // Show refresh button only if a folder is selected
            if self.state.selected_folder.is_some() {
                if ui.button("🔄 Refresh")
                    .on_hover_text("Refresh image list (F5 or Ctrl+R)")
                    .clicked() {
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
                });
            }
        } else {
            ui.label("No folder selected");
        }
        
        // Display folder validation or scanning error if any
        if let Some(error) = &self.state.ui_state.folder_error {
            ui.colored_label(ui.visuals().error_fg_color, format!("⚠ {}", error));
        }
        
        ui.separator();
        
        // Lapsify Parameters
        ui.heading("Lapsify Parameters");
        
        egui::ScrollArea::vertical()
            .id_source("settings_scroll")
            .show(ui, |ui| {
                // Image Adjustments
                ui.collapsing("Image Adjustments", |ui| {
                    let mut settings_changed = false;
                    let validation_errors = &self.state.ui_state.validation_errors;
                    
                    // Exposure
                    if Self::show_array_input(ui, "Exposure", &mut self.state.settings.exposure, -3.0, 3.0, "EV", validation_errors) {
                        settings_changed = true;
                    }
                    ui.add_space(5.0);
                    
                    // Brightness
                    if Self::show_array_input(ui, "Brightness", &mut self.state.settings.brightness, -100.0, 100.0, "", validation_errors) {
                        settings_changed = true;
                    }
                    ui.add_space(5.0);
                    
                    // Contrast
                    if Self::show_array_input(ui, "Contrast", &mut self.state.settings.contrast, 0.1, 3.0, "x", validation_errors) {
                        settings_changed = true;
                    }
                    ui.add_space(5.0);
                    
                    // Saturation
                    if Self::show_array_input(ui, "Saturation", &mut self.state.settings.saturation, 0.0, 2.0, "x", validation_errors) {
                        settings_changed = true;
                    }
                    
                    if settings_changed {
                        self.state.validate_settings();
                    }
                });
                
                ui.add_space(10.0);
                
                // Crop and Positioning
                ui.collapsing("Crop and Positioning", |ui| {
                    let mut settings_changed = false;
                    let validation_errors = &self.state.ui_state.validation_errors;
                    
                    // Crop parameters (handle separately to avoid borrowing conflicts)
                    let crop_changed = {
                        let validation_errors = &self.state.ui_state.validation_errors;
                        
                        // Check for crop-related errors
                        let has_crop_errors = validation_errors.keys().any(|k| k.starts_with("crop"));
                        
                        ui.horizontal(|ui| {
                            if has_crop_errors {
                                ui.colored_label(ui.visuals().error_fg_color, "⚠");
                            }
                            ui.label("Crop:");
                            
                            let crop_enabled = self.state.settings.crop.is_some();
                            let mut enable_crop = crop_enabled;
                            
                            if ui.checkbox(&mut enable_crop, "Enable").changed() {
                                if enable_crop && !crop_enabled {
                                    // Enable crop with default values
                                    self.state.settings.crop = Some("50%:50%:25%:25%".to_string());
                                    true
                                } else if !enable_crop && crop_enabled {
                                    // Disable crop
                                    self.state.settings.crop = None;
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        }).inner
                    };
                    
                    if crop_changed {
                        settings_changed = true;
                    }
                    
                    // Show crop validation errors
                    for (key, error) in validation_errors {
                        if key == "crop" {
                            ui.indent("crop_error", |ui| {
                                ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                            });
                        }
                    }
                    
                    // Show crop input fields if enabled
                    if let Some(crop_str) = &mut self.state.settings.crop {
                        ui.indent("crop_controls", |ui| {
                            // Parse current crop values
                            let parts: Vec<&str> = crop_str.split(':').collect();
                            if parts.len() == 4 {
                                let mut width_str = parts[0].to_string();
                                let mut height_str = parts[1].to_string();
                                let mut x_str = parts[2].to_string();
                                let mut y_str = parts[3].to_string();
                                
                                ui.horizontal(|ui| {
                                    // Check for individual crop parameter errors
                                    if validation_errors.contains_key("crop_0") {
                                        ui.colored_label(ui.visuals().error_fg_color, "⚠");
                                    }
                                    ui.label("Width:");
                                    if ui.text_edit_singleline(&mut width_str).changed() {
                                        settings_changed = true;
                                    }
                                    
                                    if validation_errors.contains_key("crop_1") {
                                        ui.colored_label(ui.visuals().error_fg_color, "⚠");
                                    }
                                    ui.label("Height:");
                                    if ui.text_edit_singleline(&mut height_str).changed() {
                                        settings_changed = true;
                                    }
                                });
                                
                                ui.horizontal(|ui| {
                                    if validation_errors.contains_key("crop_2") {
                                        ui.colored_label(ui.visuals().error_fg_color, "⚠");
                                    }
                                    ui.label("X offset:");
                                    if ui.text_edit_singleline(&mut x_str).changed() {
                                        settings_changed = true;
                                    }
                                    
                                    if validation_errors.contains_key("crop_3") {
                                        ui.colored_label(ui.visuals().error_fg_color, "⚠");
                                    }
                                    ui.label("Y offset:");
                                    if ui.text_edit_singleline(&mut y_str).changed() {
                                        settings_changed = true;
                                    }
                                });
                                
                                // Show individual crop parameter errors
                                for i in 0..4 {
                                    if let Some(error) = validation_errors.get(&format!("crop_{}", i)) {
                                        ui.indent("crop_param_error", |ui| {
                                            ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                                        });
                                    }
                                }
                                
                                if settings_changed {
                                    *crop_str = format!("{}:{}:{}:{}", width_str, height_str, x_str, y_str);
                                }
                                
                                ui.label("Format: width:height:x:y (use % for percentages)");
                                ui.label("Example: 1920:1080:100:50 or 80%:60%:10%:20%");
                            }
                        });
                    }
                    
                    ui.add_space(5.0);
                    
                    // Offset X
                    if Self::show_array_input(ui, "Offset X", &mut self.state.settings.offset_x, -1000.0, 1000.0, "px", validation_errors) {
                        settings_changed = true;
                    }
                    ui.add_space(5.0);
                    
                    // Offset Y
                    if Self::show_array_input(ui, "Offset Y", &mut self.state.settings.offset_y, -1000.0, 1000.0, "px", validation_errors) {
                        settings_changed = true;
                    }
                    
                    if settings_changed {
                        self.state.validate_settings();
                    }
                });
                
                ui.add_space(10.0);
                
                // Output Settings
                ui.collapsing("Output Settings", |ui| {
                    let validation_errors = &self.state.ui_state.validation_errors;
                    let mut settings_changed = false;
                    
                    // Format selection
                    ui.horizontal(|ui| {
                        if validation_errors.contains_key("format") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Format:");
                        egui::ComboBox::from_id_source("format_combo")
                            .selected_text(&self.state.settings.format)
                            .show_ui(ui, |ui| {
                                let formats = ["mp4", "mov", "avi", "jpg", "png", "tiff"];
                                for format in formats {
                                    if ui.selectable_value(&mut self.state.settings.format, format.to_string(), format).changed() {
                                        settings_changed = true;
                                    }
                                }
                            });
                    });
                    
                    // Show format validation errors
                    for (key, error) in validation_errors {
                        if key == "format" || key.contains("format_") {
                            ui.indent("format_error", |ui| {
                                ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                            });
                        }
                    }
                    ui.add_space(5.0);
                    
                    // FPS
                    ui.horizontal(|ui| {
                        if validation_errors.contains_key("fps") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("FPS:");
                        let response = ui.add(
                            egui::Slider::new(&mut self.state.settings.fps, 1..=120)
                                .step_by(1.0)
                        );
                        if response.changed() {
                            settings_changed = true;
                        }
                    });
                    
                    if let Some(error) = validation_errors.get("fps") {
                        ui.indent("fps_error", |ui| {
                            ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                        });
                    }
                    ui.add_space(5.0);
                    
                    // Quality (CRF)
                    ui.horizontal(|ui| {
                        if validation_errors.contains_key("quality") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Quality (CRF):");
                        let response = ui.add(
                            egui::Slider::new(&mut self.state.settings.quality, 0..=51)
                                .step_by(1.0)
                        );
                        if response.changed() {
                            settings_changed = true;
                        }
                        ui.label("(lower = better)");
                    });
                    
                    if let Some(error) = validation_errors.get("quality") {
                        ui.indent("quality_error", |ui| {
                            ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                        });
                    }
                    ui.add_space(5.0);
                    
                    // Resolution
                    ui.horizontal(|ui| {
                        if validation_errors.contains_key("resolution") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Resolution:");
                        let mut resolution_str = self.state.settings.resolution.clone().unwrap_or_default();
                        if ui.text_edit_singleline(&mut resolution_str).changed() {
                            self.state.settings.resolution = if resolution_str.is_empty() {
                                None
                            } else {
                                Some(resolution_str)
                            };
                            settings_changed = true;
                        }
                    });
                    
                    if let Some(error) = validation_errors.get("resolution") {
                        ui.indent("resolution_error", |ui| {
                            ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                        });
                    }
                    ui.label("Examples: 1920x1080, 4K, HD, or leave empty for original");
                    
                    if settings_changed {
                        self.state.validate_settings();
                    }
                });
                
                ui.add_space(10.0);
                
                // Processing Settings
                ui.collapsing("Processing Settings", |ui| {
                    let validation_errors = &self.state.ui_state.validation_errors;
                    let mut settings_changed = false;
                    
                    // Threads
                    ui.horizontal(|ui| {
                        if validation_errors.contains_key("threads") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Threads:");
                        let response = ui.add(
                            egui::Slider::new(&mut self.state.settings.threads, 0..=32)
                                .step_by(1.0)
                        );
                        if response.changed() {
                            settings_changed = true;
                        }
                        ui.label("(0 = auto)");
                    });
                    
                    if let Some(error) = validation_errors.get("threads") {
                        ui.indent("threads_error", |ui| {
                            ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                        });
                    }
                    ui.add_space(5.0);
                    
                    // Frame Range
                    ui.horizontal(|ui| {
                        if validation_errors.contains_key("frame_range") {
                            ui.colored_label(ui.visuals().error_fg_color, "⚠");
                        }
                        ui.label("Start Frame:");
                        let mut start_frame = self.state.settings.start_frame.unwrap_or(0);
                        if ui.add(egui::DragValue::new(&mut start_frame).range(0..=9999)).changed() {
                            self.state.settings.start_frame = if start_frame == 0 { None } else { Some(start_frame) };
                            settings_changed = true;
                        }
                    });
                    
                    ui.horizontal(|ui| {
                        ui.label("End Frame:");
                        let mut end_frame = self.state.settings.end_frame.unwrap_or(0);
                        if ui.add(egui::DragValue::new(&mut end_frame).range(0..=9999)).changed() {
                            self.state.settings.end_frame = if end_frame == 0 { None } else { Some(end_frame) };
                            settings_changed = true;
                        }
                    });
                    
                    if let Some(error) = validation_errors.get("frame_range") {
                        ui.indent("frame_range_error", |ui| {
                            ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                        });
                    }
                    ui.label("(0 = use default)");
                    
                    if settings_changed {
                        self.state.validate_settings();
                    }
                });
                
                ui.add_space(10.0);
                
                // CLI Execution
                ui.collapsing("Process Time-lapse", |ui| {
                    // Output directory selection
                    ui.horizontal(|ui| {
                        if ui.button("📁 Select Output Folder")
                            .on_hover_text("Choose where to save the generated time-lapse video")
                            .clicked() {
                            self.select_output_directory();
                        }
                    });
                    
                    if let Some(output_dir) = &self.state.ui_state.output_directory {
                        ui.horizontal(|ui| {
                            ui.label("Output:");
                            ui.label(output_dir.display().to_string());
                        });
                    } else {
                        ui.label("No output folder selected");
                    }
                    
                    ui.add_space(5.0);
                    
                    // Command preview
                    if let (Some(input_dir), Some(output_dir)) = (&self.state.selected_folder, &self.state.ui_state.output_directory) {
                        ui.collapsing("Command Preview", |ui| {
                            let command_preview = self.state.settings.generate_command_preview(input_dir, output_dir);
                            ui.horizontal_wrapped(|ui| {
                                ui.code(&command_preview);
                            });
                        });
                    }
                    
                    ui.add_space(5.0);
                    
                    // Processing status and controls
                    if self.state.processing_status.is_processing {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(&self.state.processing_status.status_message);
                        });
                        
                        // Progress bar
                        let progress = self.state.processing_status.progress;
                        ui.add(egui::ProgressBar::new(progress).text(format!("{:.1}%", progress * 100.0)));
                        
                        // Frame progress
                        if self.state.processing_status.total_frames > 0 {
                            ui.label(format!("Frame {} of {}", 
                                self.state.processing_status.current_frame,
                                self.state.processing_status.total_frames));
                        }
                        
                        // Cancel button
                        if ui.button("❌ Cancel Processing")
                            .on_hover_text("Stop the current time-lapse generation")
                            .clicked() {
                            self.cancel_cli_execution();
                        }
                    } else {
                        // Execute button
                        let can_execute = self.state.selected_folder.is_some() 
                            && self.state.ui_state.output_directory.is_some()
                            && !self.state.images.is_empty()
                            && self.state.ui_state.validation_errors.is_empty();
                        
                        let cli_available = self.state.check_lapsify_availability();
                        let can_execute_with_cli = can_execute && cli_available;
                        
                        ui.add_enabled_ui(can_execute_with_cli, |ui| {
                            let button_text = if !cli_available {
                                "🚀 Start Processing (CLI Not Available)"
                            } else {
                                "🚀 Start Processing"
                            };
                            
                            if ui.button(button_text)
                                .on_hover_text("Start generating time-lapse video (Ctrl+Enter)")
                                .clicked() {
                                if !cli_available {
                                    self.state.show_modal_error(
                                        "Lapsify CLI Not Found".to_string(),
                                        "The lapsify command-line tool could not be found. Please ensure it is installed and available in your system PATH.".to_string(),
                                    );
                                } else {
                                    match self.execute_lapsify_cli(ui.ctx()) {
                                        Ok(()) => {
                                            // Processing started successfully
                                        }
                                        Err(error) => {
                                            self.state.processing_status.error_message = Some(error.clone());
                                            self.state.add_error_notification(
                                                format!("Failed to start processing: {}", error),
                                                ErrorType::Error,
                                                false,
                                            );
                                        }
                                    }
                                }
                            }
                        });
                        
                        if !can_execute {
                            ui.label("⚠ Requirements:");
                            if self.state.selected_folder.is_none() {
                                ui.label("• Select input folder");
                            }
                            if self.state.ui_state.output_directory.is_none() {
                                ui.label("• Select output folder");
                            }
                            if self.state.images.is_empty() {
                                ui.label("• Input folder must contain images");
                            }
                            if !self.state.ui_state.validation_errors.is_empty() {
                                ui.label("• Fix validation errors");
                            }
                        }
                    }
                    
                    // Show processing results
                    if let Some(error) = &self.state.processing_status.error_message {
                        ui.add_space(5.0);
                        ui.colored_label(ui.visuals().error_fg_color, format!("❌ Error: {}", error));
                    }
                    
                    if let Some(output_path) = &self.state.processing_status.output_path {
                        ui.add_space(5.0);
                        ui.colored_label(ui.visuals().selection.bg_fill, "✅ Processing completed!");
                        ui.horizontal(|ui| {
                            ui.label("Output:");
                            ui.label(output_path.display().to_string());
                        });
                        
                        if ui.button("📁 Open Output Folder").clicked() {
                            if let Some(parent) = output_path.parent() {
                                let _ = std::process::Command::new("open")
                                    .arg(parent)
                                    .spawn();
                            }
                        }
                    }
                });
                
                ui.add_space(10.0);
                
                // Settings Management
                ui.collapsing("Settings Management", |ui| {
                    // Presets
                    ui.label("Presets:");
                    let mut selected_preset = None;
                    egui::ComboBox::from_id_source("presets_combo")
                        .selected_text("Select Preset")
                        .show_ui(ui, |ui| {
                            for (i, preset) in self.state.settings_presets.iter().enumerate() {
                                if ui.selectable_label(false, &preset.name).clicked() {
                                    selected_preset = Some(i);
                                }
                            }
                        });
                    
                    if let Some(preset_index) = selected_preset {
                        self.apply_preset(preset_index);
                    }
                    
                    ui.add_space(5.0);
                    
                    // Save/Load
                    ui.horizontal(|ui| {
                        if ui.button("💾 Save Settings")
                            .on_hover_text("Save current settings to file (Ctrl+S)")
                            .clicked() {
                            match self.save_settings_to_file() {
                                Ok(_) => {
                                    self.state.add_error_notification(
                                        "Settings saved successfully".to_string(),
                                        ErrorType::Info,
                                        true,
                                    );
                                }
                                Err(error) => {
                                    self.state.add_error_notification(
                                        format!("Failed to save settings: {}", error),
                                        ErrorType::Error,
                                        false,
                                    );
                                }
                            }
                        }
                        if ui.button("📁 Load Settings")
                            .on_hover_text("Load settings from file (Ctrl+L)")
                            .clicked() {
                            match self.load_settings_from_file() {
                                Ok(_) => {
                                    self.state.add_error_notification(
                                        "Settings loaded successfully".to_string(),
                                        ErrorType::Info,
                                        true,
                                    );
                                }
                                Err(error) => {
                                    self.state.add_error_notification(
                                        format!("Failed to load settings: {}", error),
                                        ErrorType::Error,
                                        false,
                                    );
                                }
                            }
                        }
                    });
                    
                    ui.horizontal(|ui| {
                        if ui.button("↺ Reset to Defaults").clicked() {
                            self.state.settings = LapsifySettings::default();
                            self.state.validate_settings();
                        }
                        if ui.button("💾 Save as Preset").clicked() {
                            // TODO: Show dialog for preset name/description
                            self.save_as_preset(
                                "Custom Preset".to_string(),
                                "User-defined preset".to_string()
                            );
                        }
                    });
                    
                    ui.add_space(5.0);
                    ui.label("💡 Tip: Presets are automatically saved and restored between sessions.");
                });
                
                ui.separator();
                
                // Help button
                ui.horizontal(|ui| {
                    if ui.button("❓ Help")
                        .on_hover_text("Show keyboard shortcuts and help (F1)")
                        .clicked() {
                        self.state.ui_state.show_help_dialog = true;
                    }
                });
                
                ui.add_space(10.0);
                
                // Validation Summary
                if !self.state.ui_state.validation_errors.is_empty() {
                    ui.add_space(10.0);
                    
                    // Count different types of errors
                    let mut error_count = 0;
                    let mut warning_count = 0;
                    
                    for (key, _) in &self.state.ui_state.validation_errors {
                        if key.contains("array_length") || key.contains("format_") {
                            warning_count += 1;
                        } else {
                            error_count += 1;
                        }
                    }
                    
                    let total_issues = error_count + warning_count;
                    let header_text = if error_count > 0 {
                        format!("⚠ {} Validation Issues ({} errors, {} warnings)", total_issues, error_count, warning_count)
                    } else {
                        format!("⚠ {} Validation Warnings", warning_count)
                    };
                    
                    ui.collapsing(header_text, |ui| {
                        // Group errors by category
                        let mut parameter_errors = Vec::new();
                        let mut format_warnings = Vec::new();
                        let mut array_warnings = Vec::new();
                        let mut other_errors = Vec::new();
                        
                        for (field, error) in &self.state.ui_state.validation_errors {
                            if field.contains("array_length") {
                                array_warnings.push((field, error));
                            } else if field.contains("format_") {
                                format_warnings.push((field, error));
                            } else if field.contains("[") || field.contains("crop_") {
                                parameter_errors.push((field, error));
                            } else {
                                other_errors.push((field, error));
                            }
                        }
                        
                        // Display parameter errors
                        if !parameter_errors.is_empty() {
                            ui.label("Parameter Range Errors:");
                            ui.indent("param_errors", |ui| {
                                for (_field, error) in parameter_errors {
                                    ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                                }
                            });
                            ui.add_space(5.0);
                        }
                        
                        // Display other errors
                        if !other_errors.is_empty() {
                            ui.label("Configuration Errors:");
                            ui.indent("other_errors", |ui| {
                                for (_field, error) in other_errors {
                                    ui.colored_label(ui.visuals().error_fg_color, format!("• {}", error));
                                }
                            });
                            ui.add_space(5.0);
                        }
                        
                        // Display format warnings
                        if !format_warnings.is_empty() {
                            ui.label("Format Compatibility Warnings:");
                            ui.indent("format_warnings", |ui| {
                                for (_field, error) in format_warnings {
                                    ui.colored_label(ui.visuals().warn_fg_color, format!("• {}", error));
                                }
                            });
                            ui.add_space(5.0);
                        }
                        
                        // Display array length warnings
                        if !array_warnings.is_empty() {
                            ui.label("Animation Array Warnings:");
                            ui.indent("array_warnings", |ui| {
                                for (_field, error) in array_warnings {
                                    ui.colored_label(ui.visuals().warn_fg_color, format!("• {}", error));
                                }
                            });
                        }
                        
                        ui.add_space(5.0);
                        ui.separator();
                        ui.label("💡 Tip: Fix errors before processing. Warnings are informational.");
                    });
                }
                
                // Development Tools (collapsible)
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
                    
                    // Show thumbnail cache statistics
                    ui.separator();
                    let cache = &self.state.ui_state.thumbnail_cache;
                    ui.label(format!("Cache: {}/{} thumbnails", 
                        cache.entries.len(), cache.max_entries));
                    ui.label(format!("Memory: {:.1}/{} MB", 
                        cache.memory_usage_mb(), cache.max_memory_mb));
                    
                    ui.horizontal(|ui| {
                        if ui.button("Load Visible Thumbnails").clicked() {
                            self.load_visible_thumbnails(ui.ctx());
                        }
                        if ui.button("Clear Cache").clicked() {
                            self.state.ui_state.thumbnail_cache.clear();
                            for image in &mut self.state.images {
                                image.thumbnail = None;
                            }
                        }
                    });
                });
            });
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
                    
                    // Show cache info
                    let cached_count = self.state.ui_state.thumbnail_cache.entries.len();
                    ui.label(format!("({} cached)", cached_count));
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
        
        // Thumbnail carousel
        if self.state.images.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No images found in selected folder.");
            });
        } else {
            // Navigation info
            ui.horizontal(|ui| {
                if let Some(index) = self.state.selected_image_index {
                    ui.label(format!("Image {} of {}", index + 1, self.state.images.len()));
                    
                    // Navigation buttons
                    if ui.button("◀ Prev")
                        .on_hover_text("Previous image (Left arrow)")
                        .clicked() && index > 0 {
                        self.state.select_image(index - 1);
                    }
                    if ui.button("Next ▶")
                        .on_hover_text("Next image (Right arrow)")
                        .clicked() && index < self.state.images.len() - 1 {
                        self.state.select_image(index + 1);
                    }
                } else {
                    ui.label(format!("{} images loaded", self.state.images.len()));
                }
            });
            
            ui.separator();
            
            // Collect click events to handle after the loop
            let mut clicked_image_index: Option<usize> = None;
            
            // Horizontal scrollable thumbnail strip
            let scroll_area_response = egui::ScrollArea::horizontal()
                .id_source("thumbnail_carousel")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(CAROUSEL_PADDING);
                        
                        for (i, image_info) in self.state.images.iter().enumerate() {
                            let is_selected = self.state.selected_image_index == Some(i);
                            
                            // Draw thumbnail or placeholder
                            let response = if let Some(thumbnail_texture) = &image_info.thumbnail {
                                // Draw actual thumbnail
                                let image_response = ui.add(
                                    egui::Image::from_texture(thumbnail_texture)
                                        .max_size(egui::vec2(THUMBNAIL_SIZE, THUMBNAIL_SIZE))
                                        .rounding(egui::Rounding::same(4.0))
                                );
                                
                                // Add selection border
                                if is_selected {
                                    ui.painter().rect_stroke(
                                        image_response.rect.expand(2.0),
                                        egui::Rounding::same(6.0),
                                        egui::Stroke::new(3.0, ui.visuals().selection.bg_fill)
                                    );
                                }
                                
                                image_response
                            } else {
                                // Draw placeholder
                                let placeholder_response = ui.allocate_response(
                                    egui::vec2(THUMBNAIL_SIZE, THUMBNAIL_SIZE),
                                    egui::Sense::click()
                                );
                                
                                let fill_color = if is_selected {
                                    ui.visuals().selection.bg_fill
                                } else {
                                    ui.visuals().window_fill
                                };
                                
                                ui.painter().rect_filled(
                                    placeholder_response.rect,
                                    egui::Rounding::same(4.0),
                                    fill_color
                                );
                                
                                ui.painter().rect_stroke(
                                    placeholder_response.rect,
                                    egui::Rounding::same(4.0),
                                    egui::Stroke::new(1.0, ui.visuals().text_color())
                                );
                                
                                // Show loading indicator or filename
                                let text = match self.state.ui_state.thumbnail_load_states.get(&image_info.path) {
                                    Some(ThumbnailLoadState::Loading) => "⏳".to_string(),
                                    Some(ThumbnailLoadState::Error(_)) => "❌".to_string(),
                                    _ => {
                                        // Show filename or image number
                                        if let Some(filename) = image_info.path.file_stem() {
                                            filename.to_string_lossy().chars().take(8).collect()
                                        } else {
                                            format!("{}", i + 1)
                                        }
                                    }
                                };
                                
                                ui.painter().text(
                                    placeholder_response.rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    text,
                                    egui::FontId::proportional(12.0),
                                    ui.visuals().text_color()
                                );
                                
                                placeholder_response
                            };
                            
                            // Handle click
                            if response.clicked() {
                                clicked_image_index = Some(i);
                            }
                            
                            // Show tooltip with image info
                            if response.hovered() {
                                response.on_hover_ui(|ui| {
                                    ui.label(format!("Image {}", i + 1));
                                    ui.label(image_info.path.file_name().unwrap_or_default().to_string_lossy());
                                    ui.label(format!("{}x{}", image_info.metadata.width, image_info.metadata.height));
                                    ui.label(format!("{}", image_info.metadata.format));
                                    if let Some(modified) = image_info.metadata.modified {
                                        if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                                            ui.label(format!("Modified: {}", duration.as_secs()));
                                        }
                                    }
                                });
                            }
                            
                            ui.add_space(THUMBNAIL_SPACING);
                        }
                        
                        ui.add_space(CAROUSEL_PADDING);
                    });
                });
            
            // Handle click events after the loop
            if let Some(index) = clicked_image_index {
                self.state.select_image(index);
            }
            
            // Calculate visible thumbnails and trigger lazy loading
            let scroll_rect = scroll_area_response.inner_rect;
            let scroll_offset = scroll_area_response.state.offset.x;
            self.calculate_visible_thumbnails(scroll_rect, scroll_offset);
            self.load_visible_carousel_thumbnails(ui.ctx());
            
            // Show carousel statistics
            ui.horizontal(|ui| {
                let (start, end) = self.state.ui_state.visible_thumbnail_range;
                ui.label(format!("Visible: {}-{}", start + 1, end));
                ui.label(format!("Cache: {:.1}MB", self.state.ui_state.thumbnail_cache.memory_usage_mb()));
            });
        }
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
        
        // Image viewer controls and info
        if let Some(selected_index) = self.state.selected_image_index {
            // Collect UI actions to handle after borrowing
            let mut zoom_in = false;
            let mut zoom_out = false;
            let mut reset_view = false;
            let mut load_full_image = false;
            
            if let Some(selected_image) = self.state.get_selected_image() {
                // Image info and controls
                ui.horizontal(|ui| {
                    ui.label("Selected:");
                    ui.label(selected_image.path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy());
                    
                    ui.separator();
                    
                    // Zoom controls
                    ui.label(format!("Zoom: {:.1}%", self.state.ui_state.zoom_level * 100.0));
                    if ui.button("🔍+")
                        .on_hover_text("Zoom in (+)")
                        .clicked() {
                        zoom_in = true;
                    }
                    if ui.button("🔍-")
                        .on_hover_text("Zoom out (-)")
                        .clicked() {
                        zoom_out = true;
                    }
                    if ui.button("↺ Reset")
                        .on_hover_text("Reset zoom and pan (Ctrl+0)")
                        .clicked() {
                        reset_view = true;
                    }
                    
                    ui.separator();
                    
                    // Load full image button
                    if selected_image.full_image.is_none() {
                        if ui.button("🖼 Load Full Image").clicked() {
                            load_full_image = true;
                        }
                    } else {
                        ui.colored_label(ui.visuals().selection.bg_fill, "✓ Full image loaded");
                    }
                });
                
                ui.horizontal(|ui| {
                    ui.label(format!("Size: {}x{}", 
                        selected_image.metadata.width, 
                        selected_image.metadata.height));
                    ui.label(format!("Format: {}", selected_image.metadata.format));
                    ui.label(format!("File: {:.1} MB", 
                        selected_image.metadata.file_size as f64 / 1_048_576.0));
                });
            }
            
            // Handle UI actions after borrowing
            if zoom_in {
                self.handle_zoom(1.0);
            }
            if zoom_out {
                self.handle_zoom(-1.0);
            }
            if reset_view {
                self.reset_view();
            }
            if load_full_image {
                self.state.load_full_image_sync(selected_index, ui.ctx());
            }
                
            // Main image display area
            let available_rect = ui.available_rect_before_wrap();
            let image_area = egui::Rect::from_min_size(
                available_rect.min,
                available_rect.size() - egui::vec2(0.0, 20.0) // Leave space for status
            );
            
            // Handle mouse wheel for zooming (collect scroll delta first)
            let scroll_delta = ui.input(|i| i.raw_scroll_delta);
            let should_zoom = scroll_delta.y != 0.0 && ui.rect_contains_pointer(image_area);
            let zoom_delta = scroll_delta.y * 0.01;
            
            // Get selected image info for display
            if let Some(selected_image) = self.state.get_selected_image() {
                // Create a scroll area for pan functionality
                let _scroll_response = egui::ScrollArea::both()
                    .id_source("image_viewer_scroll")
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, _viewport| {
                        // Display the image or placeholder
                        if let Some(full_image_texture) = &selected_image.full_image {
                            // Display full-size image
                            let image_size = egui::Vec2::new(
                                full_image_texture.size()[0] as f32,
                                full_image_texture.size()[1] as f32
                            );
                            
                            let display_size = self.calculate_image_display_size(image_size, image_area.size());
                            
                            // Center the image in the available space
                            let image_pos = if display_size.x < image_area.width() && display_size.y < image_area.height() {
                                // Image fits, center it
                                egui::pos2(
                                    (image_area.width() - display_size.x) * 0.5,
                                    (image_area.height() - display_size.y) * 0.5
                                )
                            } else {
                                // Image is larger, position at origin for scrolling
                                egui::pos2(0.0, 0.0)
                            };
                            
                            let image_rect = egui::Rect::from_min_size(image_pos, display_size);
                            
                            // Allocate space for the image
                            ui.allocate_exact_size(display_size, egui::Sense::click_and_drag());
                            
                            // Draw the image
                            ui.painter().image(
                                full_image_texture.id(),
                                image_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE
                            );
                            
                        } else if let Some(thumbnail_texture) = &selected_image.thumbnail {
                            // Display thumbnail as placeholder
                            let thumbnail_size = egui::Vec2::new(
                                thumbnail_texture.size()[0] as f32,
                                thumbnail_texture.size()[1] as f32
                            );
                            
                            let display_size = self.calculate_image_display_size(thumbnail_size, image_area.size());
                            
                            // Center the thumbnail
                            let image_pos = egui::pos2(
                                (image_area.width() - display_size.x) * 0.5,
                                (image_area.height() - display_size.y) * 0.5
                            );
                            
                            let image_rect = egui::Rect::from_min_size(image_pos, display_size);
                            
                            // Allocate space
                            ui.allocate_exact_size(image_area.size(), egui::Sense::click());
                            
                            // Draw thumbnail
                            ui.painter().image(
                                thumbnail_texture.id(),
                                image_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE
                            );
                            
                            // Add "thumbnail" indicator
                            ui.painter().text(
                                image_rect.right_bottom() - egui::vec2(10.0, 10.0),
                                egui::Align2::RIGHT_BOTTOM,
                                "Thumbnail",
                                egui::FontId::proportional(12.0),
                                ui.visuals().text_color()
                            );
                            
                        } else {
                            // No image loaded - show placeholder
                            ui.allocate_exact_size(image_area.size(), egui::Sense::click());
                            
                            let placeholder_rect = egui::Rect::from_center_size(
                                image_area.center(),
                                egui::vec2(200.0, 150.0)
                            );
                            
                            ui.painter().rect_stroke(
                                placeholder_rect,
                                5.0,
                                egui::Stroke::new(2.0, ui.visuals().text_color())
                            );
                            
                            ui.painter().text(
                                placeholder_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "Loading image...\nClick 'Load Full Image' above",
                                egui::FontId::proportional(14.0),
                                ui.visuals().text_color()
                            );
                        }
                    });
                
                // Status bar (collect display info first)
                let display_info = if let Some(full_image) = &selected_image.full_image {
                    Some((full_image.size()[0] as f32, full_image.size()[1] as f32))
                } else {
                    None
                };
                
                ui.horizontal(|ui| {
                    ui.label(format!("Zoom: {:.1}%", self.state.ui_state.zoom_level * 100.0));
                    if let Some((width, height)) = display_info {
                        ui.label(format!("Display: {:.0}x{:.0}px", 
                            width * self.state.ui_state.zoom_level,
                            height * self.state.ui_state.zoom_level));
                    }
                    ui.label(format!("Viewer: {:.0}x{:.0}px", image_area.width(), image_area.height()));
                });
            } else {
                // No image loaded - show placeholder
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(50.0);
                        ui.label("No image data available");
                        ui.label("Try selecting a different image");
                    });
                });
            }
            
            // Handle zoom after image display (outside of selected_image scope)
            if should_zoom {
                self.handle_zoom(zoom_delta);
            }
        } else {
            // No image selected
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(50.0);
                    ui.label("No image selected");
                    ui.label("Click on a thumbnail in the carousel below to view an image");
                    ui.add_space(20.0);
                    
                    // Show some helpful info
                    if !self.state.images.is_empty() {
                        ui.label(format!("📁 {} images available", self.state.images.len()));
                        ui.label("Use the carousel below to browse images");
                    }
                });
            });
        }
    }
    
    /// Show error notifications as toast-style messages
    fn show_error_notifications(&mut self, ctx: &egui::Context) {
        let notifications = self.state.ui_state.error_notifications.clone();
        let mut to_remove = Vec::new();
        
        for (index, notification) in notifications.iter().enumerate() {
            let (bg_color, text_color) = match notification.error_type {
                ErrorType::Info => (egui::Color32::from_rgb(70, 130, 180), egui::Color32::WHITE),
                ErrorType::Warning => (egui::Color32::from_rgb(255, 165, 0), egui::Color32::BLACK),
                ErrorType::Error => (egui::Color32::from_rgb(220, 20, 60), egui::Color32::WHITE),
                ErrorType::Critical => (egui::Color32::from_rgb(139, 0, 0), egui::Color32::WHITE),
            };
            
            let y_offset = 10.0 + (index as f32 * 60.0);
            
            egui::Window::new(format!("notification_{}", index))
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::RIGHT_TOP, egui::Vec2::new(-10.0, y_offset))
                .fixed_size(egui::Vec2::new(350.0, 50.0))
                .frame(egui::Frame::window(&ctx.style()).fill(bg_color))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let icon = match notification.error_type {
                            ErrorType::Info => "ℹ️",
                            ErrorType::Warning => "⚠️",
                            ErrorType::Error => "❌",
                            ErrorType::Critical => "🚨",
                        };
                        
                        ui.colored_label(text_color, icon);
                        ui.colored_label(text_color, &notification.message);
                        
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                to_remove.push(index);
                            }
                        });
                    });
                });
        }
        
        // Remove dismissed notifications
        for &index in to_remove.iter().rev() {
            self.state.ui_state.error_notifications.remove(index);
        }
    }
    
    /// Show modal dialog for critical errors
    fn show_modal_dialog(&mut self, ctx: &egui::Context) {
        if !self.state.ui_state.modal_dialog.is_open {
            return;
        }
        
        let title = self.state.ui_state.modal_dialog.title.clone();
        let message = self.state.ui_state.modal_dialog.message.clone();
        let dialog_type = self.state.ui_state.modal_dialog.dialog_type.clone();
        
        let mut should_close = false;
        
        egui::Window::new(&title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let icon = match dialog_type {
                        DialogType::Error => "❌",
                        DialogType::Confirmation => "❓",
                        DialogType::Info => "ℹ️",
                    };
                    
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(icon).size(32.0));
                    ui.add_space(10.0);
                    
                    ui.label(&message);
                    ui.add_space(20.0);
                    
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            should_close = true;
                        }
                        
                        if dialog_type == DialogType::Confirmation {
                            if ui.button("Cancel").clicked() {
                                should_close = true;
                            }
                        }
                    });
                });
            });
        
        if should_close {
            self.state.ui_state.modal_dialog.is_open = false;
        }
    }
    
    /// Handle keyboard shortcuts for common actions
    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input_mut(|i| {
            // Folder selection: Ctrl/Cmd + O
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::O)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::MAC_CMD, egui::Key::O)) {
                self.select_folder();
            }
            
            // Refresh images: F5 or Ctrl/Cmd + R
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F5)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::R)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::MAC_CMD, egui::Key::R)) {
                if self.state.selected_folder.is_some() {
                    self.refresh_images();
                }
            }
            
            // Save settings: Ctrl/Cmd + S
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::S)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::MAC_CMD, egui::Key::S)) {
                if let Err(error) = self.save_settings_to_file() {
                    self.state.add_error_notification(
                        format!("Failed to save settings: {}", error),
                        ErrorType::Error,
                        false,
                    );
                } else {
                    self.state.add_error_notification(
                        "Settings saved successfully".to_string(),
                        ErrorType::Info,
                        true,
                    );
                }
            }
            
            // Load settings: Ctrl/Cmd + L
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::L)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::MAC_CMD, egui::Key::L)) {
                if let Err(error) = self.load_settings_from_file() {
                    self.state.add_error_notification(
                        format!("Failed to load settings: {}", error),
                        ErrorType::Error,
                        false,
                    );
                } else {
                    self.state.add_error_notification(
                        "Settings loaded successfully".to_string(),
                        ErrorType::Info,
                        true,
                    );
                }
            }
            
            // Image navigation: Arrow keys
            if !self.state.images.is_empty() {
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
                    if let Some(current) = self.state.selected_image_index {
                        if current > 0 {
                            self.state.select_image(current - 1);
                        }
                    }
                }
                
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
                    if let Some(current) = self.state.selected_image_index {
                        if current < self.state.images.len() - 1 {
                            self.state.select_image(current + 1);
                        }
                    } else if !self.state.images.is_empty() {
                        self.state.select_image(0);
                    }
                }
                
                // Home/End for first/last image
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Home)) {
                    self.state.select_image(0);
                }
                
                if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::End)) {
                    self.state.select_image(self.state.images.len() - 1);
                }
            }
            
            // Zoom controls: Plus/Minus
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Equals)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Equals)) {
                self.state.ui_state.zoom_level = (self.state.ui_state.zoom_level * 1.2).min(5.0);
            }
            
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Minus)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Minus)) {
                self.state.ui_state.zoom_level = (self.state.ui_state.zoom_level / 1.2).max(0.1);
            }
            
            // Reset zoom: Ctrl/Cmd + 0
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Num0)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::MAC_CMD, egui::Key::Num0)) {
                self.state.ui_state.zoom_level = 1.0;
                self.state.ui_state.pan_offset = egui::Vec2::ZERO;
            }
            
            // Start processing: Ctrl/Cmd + Enter
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Enter)) ||
               i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::MAC_CMD, egui::Key::Enter)) {
                if !self.state.processing_status.is_processing && 
                   !self.state.images.is_empty() && 
                   self.state.check_lapsify_availability() {
                    match self.execute_lapsify_cli(ctx) {
                        Ok(()) => {
                            // Processing started successfully
                        }
                        Err(error) => {
                            self.state.add_error_notification(
                                format!("Failed to start processing: {}", error),
                                ErrorType::Error,
                                false,
                            );
                        }
                    }
                }
            }
            
            // Show help: F1
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F1)) {
                self.state.ui_state.show_help_dialog = true;
            }
            
            // Escape to close modal dialogs
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Escape)) {
                if self.state.ui_state.modal_dialog.is_open {
                    self.state.ui_state.modal_dialog.is_open = false;
                }
                if self.state.ui_state.show_help_dialog {
                    self.state.ui_state.show_help_dialog = false;
                }
            }
        });
    }
    
    /// Show help dialog with keyboard shortcuts
    fn show_help_dialog(&mut self, ctx: &egui::Context) {
        if !self.state.ui_state.show_help_dialog {
            return;
        }
        
        let mut should_close = false;
        
        egui::Window::new("Keyboard Shortcuts")
            .collapsible(false)
            .resizable(true)
            .default_size(egui::Vec2::new(500.0, 600.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.heading("Lapsify GUI - Keyboard Shortcuts");
                ui.separator();
                
                ui.columns(2, |columns| {
                    columns[0].heading("Action");
                    columns[1].heading("Shortcut");
                });
                
                ui.separator();
                
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.columns(2, |columns| {
                        // File operations
                        columns[0].label("Select Folder");
                        columns[1].label("Ctrl+O / Cmd+O");
                        
                        columns[0].label("Refresh Images");
                        columns[1].label("F5 / Ctrl+R / Cmd+R");
                        
                        columns[0].label("Save Settings");
                        columns[1].label("Ctrl+S / Cmd+S");
                        
                        columns[0].label("Load Settings");
                        columns[1].label("Ctrl+L / Cmd+L");
                        
                        columns[0].separator();
                        columns[1].separator();
                        
                        // Image navigation
                        columns[0].label("Previous Image");
                        columns[1].label("Left Arrow");
                        
                        columns[0].label("Next Image");
                        columns[1].label("Right Arrow");
                        
                        columns[0].label("First Image");
                        columns[1].label("Home");
                        
                        columns[0].label("Last Image");
                        columns[1].label("End");
                        
                        columns[0].separator();
                        columns[1].separator();
                        
                        // Zoom controls
                        columns[0].label("Zoom In");
                        columns[1].label("+ / Ctrl++");
                        
                        columns[0].label("Zoom Out");
                        columns[1].label("- / Ctrl+-");
                        
                        columns[0].label("Reset Zoom");
                        columns[1].label("Ctrl+0 / Cmd+0");
                        
                        columns[0].separator();
                        columns[1].separator();
                        
                        // Processing
                        columns[0].label("Start Processing");
                        columns[1].label("Ctrl+Enter / Cmd+Enter");
                        
                        columns[0].separator();
                        columns[1].separator();
                        
                        // General
                        columns[0].label("Show Help");
                        columns[1].label("F1");
                        
                        columns[0].label("Close Dialog");
                        columns[1].label("Escape");
                    });
                });
                
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        should_close = true;
                    }
                    
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("Press F1 anytime to show this help");
                    });
                });
            });
        
        if should_close {
            self.state.ui_state.show_help_dialog = false;
        }
    }
}

impl eframe::App for LapsifyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Initialize on first run
        if !self.initialized {
            // Load session state
            if let Err(error) = self.state.load_session() {
                println!("Failed to load session: {}", error);
            }
            
            // Load presets
            if let Err(error) = self.state.load_presets() {
                println!("Failed to load presets: {}", error);
            }
            
            // Apply window state if available
            if let (Some((width, height)), Some((x, y))) = (self.state.ui_state.window_size, self.state.ui_state.window_position) {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(width, height)));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::Pos2::new(x, y)));
            }
            
            // Check lapsify CLI availability
            self.state.check_lapsify_availability();
            
            // Rescan images if folder was restored
            if self.state.selected_folder.is_some() {
                let _ = self.state.scan_images();
            }
            
            self.initialized = true;
        }
        
        // Update processing status from background thread
        self.update_processing_status();
        
        // Update window state for persistence
        self.update_window_state(ctx);
        
        // Apply responsive layout
        let screen_size = ctx.screen_rect().size();
        self.apply_responsive_layout(screen_size);
        
        // Left sidebar panel for settings with responsive constraints
        let min_sidebar = 250.0_f32.max(screen_size.x * 0.15);
        let max_sidebar = 400.0_f32.min(screen_size.x * 0.35);
        
        let sidebar_response = egui::SidePanel::left("settings_sidebar")
            .resizable(true)
            .default_width(self.state.ui_state.sidebar_width)
            .width_range(min_sidebar..=max_sidebar)
            .show(ctx, |ui| {
                self.show_settings_sidebar(ui);
            });
        
        // Update stored sidebar width if it was resized
        if (self.state.ui_state.sidebar_width - sidebar_response.response.rect.width()).abs() > 1.0 {
            self.state.ui_state.sidebar_width = sidebar_response.response.rect.width();
        }

        // Bottom panel for thumbnail carousel with responsive constraints
        let min_carousel = 100.0_f32.max(screen_size.y * 0.1);
        let max_carousel = 250.0_f32.min(screen_size.y * 0.3);
        
        let carousel_response = egui::TopBottomPanel::bottom("thumbnail_carousel")
            .resizable(true)
            .default_height(self.state.ui_state.carousel_height)
            .height_range(min_carousel..=max_carousel)
            .show(ctx, |ui| {
                self.show_thumbnail_carousel(ui);
            });
        
        // Update stored carousel height if it was resized
        if (self.state.ui_state.carousel_height - carousel_response.response.rect.height()).abs() > 1.0 {
            self.state.ui_state.carousel_height = carousel_response.response.rect.height();
        }

        // Central panel for main image viewer
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_main_viewer(ui);
        });
        
        // Handle keyboard shortcuts
        self.handle_keyboard_shortcuts(ctx);
        
        // Performance optimizations
        self.state.update_frame_timing();
        
        // Process background loading (limit to 1 per frame for smooth UI)
        let loaded_something = self.state.process_background_loading(ctx);
        
        // Only request repaint if we loaded something or if processing is active
        if loaded_something || self.state.processing_status.is_processing {
            ctx.request_repaint();
        }
        
        // Clean up old notifications
        self.state.cleanup_notifications();
        
        // Show error notifications
        self.show_error_notifications(ctx);
        
        // Show modal dialog if open
        self.show_modal_dialog(ctx);
        
        // Show help dialog if open
        self.show_help_dialog(ctx);
        
        // Periodic cleanup (every 5 seconds)
        static mut LAST_CLEANUP: Option<Instant> = None;
        let should_cleanup = unsafe {
            match LAST_CLEANUP {
                None => true,
                Some(last) => last.elapsed().as_secs() > 5,
            }
        };
        
        if should_cleanup {
            self.state.cleanup_unused_textures();
            unsafe {
                LAST_CLEANUP = Some(Instant::now());
            }
        }
        
        // Save session state periodically (every 30 seconds or on significant changes)
        static mut LAST_SAVE: Option<Instant> = None;
        let should_save = unsafe {
            match LAST_SAVE {
                None => true,
                Some(last) => last.elapsed().as_secs() > 30,
            }
        };
        
        if should_save {
            if let Err(error) = self.state.save_session() {
                println!("Failed to save session: {}", error);
            }
            unsafe {
                LAST_SAVE = Some(Instant::now());
            }
        }
    }
    
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        // Save session state when app is closing
        if let Err(error) = self.state.save_session() {
            println!("Failed to save session on exit: {}", error);
        }
    }
}