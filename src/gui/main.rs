use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::fs;
use std::time::{SystemTime, Instant};
use image::{GenericImageView, DynamicImage, imageops::FilterType};
use std::thread;

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
        
        // Clear existing images and thumbnail states
        self.images.clear();
        self.selected_image_index = None;
        self.ui_state.thumbnail_cache.clear();
        self.ui_state.thumbnail_load_states.clear();
        
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

/// Thumbnail cache entry with metadata
#[derive(Clone)]
pub struct ThumbnailCacheEntry {
    pub texture: egui::TextureHandle,
    pub last_accessed: Instant,
    pub memory_size: usize,
}

/// LRU cache for thumbnails with memory management
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
}

/// Thumbnail loading state
#[derive(Clone, PartialEq)]
pub enum ThumbnailLoadState {
    NotStarted,
    Loading,
    Loaded,
    Error(String),
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
    pub thumbnail_cache: ThumbnailCache,
    pub thumbnail_load_states: HashMap<PathBuf, ThumbnailLoadState>,
    pub carousel_scroll_offset: f32,
    pub visible_thumbnail_range: (usize, usize),
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
    
    /// Load thumbnails for images visible in the carousel viewport
    fn load_visible_carousel_thumbnails(&mut self, ctx: &egui::Context) {
        let (start, end) = self.state.ui_state.visible_thumbnail_range;
        
        // Load thumbnails for visible range plus a buffer
        let buffer = 3; // Load 3 extra on each side
        let start_with_buffer = start.saturating_sub(buffer);
        let end_with_buffer = std::cmp::min(end + buffer, self.state.images.len());
        
        for i in start_with_buffer..end_with_buffer {
            if i < self.state.images.len() {
                self.state.load_thumbnail_sync(i, ctx);
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
                            
                            // Show thumbnail status
                            let thumbnail_status = if selected_image.thumbnail.is_some() {
                                "✓ Thumbnail loaded"
                            } else {
                                "⏳ Thumbnail not loaded"
                            };
                            ui.label(thumbnail_status);
                        }
                    });
                    
                    // Show thumbnail cache statistics
                    ui.collapsing("Thumbnail Cache", |ui| {
                        let cache = &self.state.ui_state.thumbnail_cache;
                        ui.label(format!("Cached: {}/{} thumbnails", 
                            cache.entries.len(), cache.max_entries));
                        ui.label(format!("Memory: {:.1}/{} MB", 
                            cache.memory_usage_mb(), cache.max_memory_mb));
                        
                        // Cache management buttons
                        ui.horizontal(|ui| {
                            if ui.button("Load Visible Thumbnails").clicked() {
                                self.load_visible_thumbnails(ui.ctx());
                            }
                            if ui.button("Clear Cache").clicked() {
                                self.state.ui_state.thumbnail_cache.clear();
                                // Clear thumbnail references in images
                                for image in &mut self.state.images {
                                    image.thumbnail = None;
                                }
                            }
                        });
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
                    if ui.button("◀ Prev").clicked() && index > 0 {
                        self.state.select_image(index - 1);
                    }
                    if ui.button("Next ▶").clicked() && index < self.state.images.len() - 1 {
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
                    if ui.button("🔍+").clicked() {
                        zoom_in = true;
                    }
                    if ui.button("🔍-").clicked() {
                        zoom_out = true;
                    }
                    if ui.button("↺ Reset").clicked() {
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