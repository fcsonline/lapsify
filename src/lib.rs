// Re-export the main processing logic for the GUI
pub mod main;
pub use main::{ImageAdjustments, apply_adjustments, is_image_file, ProcessingError}; 