use clap::{Arg, Command};
use image::{DynamicImage, ImageBuffer, Rgb, GenericImageView};
use std::path::{Path, PathBuf};
use std::fs;
use std::error::Error;
use std::process::Command as ProcessCommand;
use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::fmt;
use colored::*;
use std::time::Instant;

#[derive(Debug)]
struct ProcessingError(String);

impl fmt::Display for ProcessingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.red())
    }
}

impl std::error::Error for ProcessingError {}

impl From<Box<dyn std::error::Error>> for ProcessingError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        ProcessingError(err.to_string())
    }
}

#[derive(Debug, Clone)]
struct ImageAdjustments {
    exposure: Vec<f32>,
    brightness: Vec<f32>,
    contrast: Vec<f32>,
    saturation: Vec<f32>,
    crop: Option<String>,
}

// Implement Send and Sync for ImageAdjustments to make it thread-safe
unsafe impl Send for ImageAdjustments {}
unsafe impl Sync for ImageAdjustments {}

impl Default for ImageAdjustments {
    fn default() -> Self {
        Self {
            exposure: vec![0.0],     // EV stops (+/- values)
            brightness: vec![0.0],   // -100 to +100
            contrast: vec![1.0],     // 0.0 to 2.0 (1.0 = no change)
            saturation: vec![1.0],   // 0.0 to 2.0 (1.0 = no change)
            crop: None,               // Crop string in format "width:height:x:y"
        }
    }
}

impl ImageAdjustments {
    fn get_values_at_frame(&self, frame_index: usize, total_frames: usize) -> (f32, f32, f32, f32) {
        (
            interpolate_value(&self.exposure, frame_index, total_frames),
            interpolate_value(&self.brightness, frame_index, total_frames),
            interpolate_value(&self.contrast, frame_index, total_frames),
            interpolate_value(&self.saturation, frame_index, total_frames),
        )
    }
}

// Helper functions
fn parse_value_array(input: &str) -> Result<Vec<f32>, Box<dyn Error>> {
    input
        .split(',')
        .map(|s| s.trim().parse::<f32>())
        .collect::<Result<Vec<f32>, _>>()
        .map_err(|e| format!("Failed to parse value array: {}", e).into())
}

#[derive(Debug, Clone)]
struct CropParams {
    width: f32,
    height: f32,
    x: f32,
    y: f32,
}

fn parse_crop_string(input: &str) -> Result<CropParams, Box<dyn Error>> {
    let parts: Vec<&str> = input.split(':').collect();
    if parts.len() != 4 {
        return Err(format!("Crop string must have 4 parts (width:height:x:y), got {} parts", parts.len()).into());
    }
    
    let width = parse_crop_value(parts[0])?;
    let height = parse_crop_value(parts[1])?;
    let x = parse_crop_value(parts[2])?;
    let y = parse_crop_value(parts[3])?;
    
    Ok(CropParams { width, height, x, y })
}

fn parse_crop_value(input: &str) -> Result<f32, Box<dyn Error>> {
    let input = input.trim();
    if input.ends_with('%') {
        let percentage = input[..input.len()-1].parse::<f32>()
            .map_err(|_| format!("Invalid percentage value: {}", input))?;
        if percentage < 0.0 || percentage > 100.0 {
            return Err(format!("Percentage must be between 0 and 100: {}", input).into());
        }
        Ok(percentage)
    } else {
        let pixels = input.parse::<f32>()
            .map_err(|_| format!("Invalid pixel value: {}", input))?;
        // Allow negative values for crop offsets (they indicate offset from right/bottom)
        Ok(pixels)
    }
}



fn calculate_frame_padding(total_frames: usize) -> usize {
    // Calculate the number of digits needed for the largest frame number
    let max_frame = total_frames;
    if max_frame == 0 {
        1
    } else {
        max_frame.ilog10() as usize + 1
    }
}

fn validate_resolution_proportion(
    image_files: &[PathBuf],
    target_resolution: Option<&str>,
) -> Result<Option<(u32, u32)>, Box<dyn Error>> {
    if let Some(res) = target_resolution {
        // Get the first image to determine original dimensions
        if let Some(first_image_path) = image_files.first() {
            let img = image::open(first_image_path)?;
            let (original_width, original_height) = img.dimensions();
            
            // Parse target resolution
            let (target_width, target_height) = parse_resolution(res)?;
            
            // Calculate the actual output width to maintain aspect ratio
            // Keep the specified height, adjust width to preserve original aspect ratio
            let original_ratio = original_width as f32 / original_height as f32;
            let mut output_width = (target_height as f32 * original_ratio) as u32;
            
            // Ensure width is even for H.264 compatibility
            if output_width % 2 != 0 {
                output_width += 1;
            }
            
            // Ensure height is even for H.264 compatibility
            let mut output_height = target_height;
            if output_height % 2 != 0 {
                output_height += 1;
            }
            
            // Calculate aspect ratios for comparison
            let target_ratio = target_width as f32 / target_height as f32;
            
            // Check if aspect ratios are significantly different (within 5% tolerance)
            let ratio_difference = (original_ratio - target_ratio).abs();
            let tolerance = 0.05;
            
            if ratio_difference > tolerance {
                println!(
                    "{}: Original aspect ratio ({:.2}:1) differs from target ({:.2}:1). This may cause distortion. {}: {}x{}",
                    "Warning".yellow(),
                    original_ratio,
                    target_ratio,
                    "Output resolution".yellow(),
                    output_width,
                    output_height
                );
            } else {
                println!(
                    "{}: Aspect ratio validation passed ({:.2}:1). {}: {}x{}",
                    "Resolution".green(),
                    original_ratio,
                    "Output resolution".green(),
                    output_width,
                    output_height
                );
            }
            Ok(Some((output_width, output_height)))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

fn parse_resolution(resolution: &str) -> Result<(u32, u32), Box<dyn Error>> {
    let res_str = match resolution.to_lowercase().as_str() {
        "4k" => "3840x2160",
        "hd" | "1080p" => "1920x1080",
        "720p" => "1280x720",
        _ => resolution, // Use as-is for custom resolutions like "1920x1080"
    };
    
    let parts: Vec<&str> = res_str.split('x').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid resolution format: {}", resolution).into());
    }
    
    let width = parts[0].parse::<u32>()
        .map_err(|_| format!("Invalid width in resolution: {}", parts[0]))?;
    let height = parts[1].parse::<u32>()
        .map_err(|_| format!("Invalid height in resolution: {}", parts[1]))?;
    
    Ok((width, height))
}

fn validate_value_array(values: &[f32], name: &str, min: f32, max: f32) -> Result<(), Box<dyn Error>> {
    for (i, &value) in values.iter().enumerate() {
        if value < min || value > max {
            return Err(format!(
                "{} value at index {} ({}) is outside valid range [{}, {}]",
                name.red(), i, value, min, max
            ).into());
        }
    }
    Ok(())
}

fn interpolate_value(values: &[f32], frame_index: usize, total_frames: usize) -> f32 {
    if values.len() == 1 {
        values[0]
    } else if values.len() == 2 {
        // Linear interpolation for 2 points
        let t = frame_index as f32 / (total_frames - 1) as f32;
        values[0] + (values[1] - values[0]) * t
    } else {
        // Bezier curve interpolation for multiple points
        let t = frame_index as f32 / (total_frames - 1) as f32;
        bezier_interpolate(values, t)
    }
}

/// Bezier curve interpolation using Bernstein polynomials
/// This provides smooth, natural transitions between control points
/// Formula: B(t) = Σ(i=0 to n) C(n,i) * P_i * (1-t)^(n-i) * t^i
fn bezier_interpolate(control_points: &[f32], t: f32) -> f32 {
    let n = control_points.len() - 1;
    if n == 0 {
        return control_points[0];
    }
    
    let mut result = 0.0;
    for (i, &point) in control_points.iter().enumerate() {
        let coefficient = binomial_coefficient(n, i) as f32;
        result += coefficient * point * (1.0 - t).powi((n - i) as i32) * t.powi(i as i32);
    }
    result
}

/// Calculate binomial coefficient C(n,k) = n! / (k! * (n-k)!)
/// Used in Bezier curve interpolation for Bernstein polynomial coefficients
fn binomial_coefficient(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    if k == 0 || k == n {
        return 1;
    }
    
    let mut result = 1;
    for i in 0..k {
        result = result * (n - i) / (i + 1);
    }
    result
}

fn print_value_array(name: &str, values: &[f32], unit: &str) {
    if values.len() == 1 {
        println!("  {}: {}{}", name.green(), values[0], unit);
    } else {
        let values_str = values.iter()
            .map(|v| format!("{}{}", v, unit))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {}: [{}]", name.green(), values_str);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let matches = Command::new("timelapse-processor")
        .version("1.0")
        .about("Process time-lapse images with adjustable parameters")
        .arg(
            Arg::new("input")
                .short('i')
                .long("input")
                .value_name("DIR")
                .help("Input directory containing images")
                .required(true),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("DIR")
                .help("Output directory for processed images")
                .required(true),
        )
        .arg(
            Arg::new("exposure")
                .short('e')
                .long("exposure")
                .value_name("STOPS")
                .help("Exposure adjustment in EV stops. Single value (-3.0 to +3.0) or comma-separated array (e.g., '0.0,1.5,-0.5')")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("brightness")
                .short('b')
                .long("brightness")
                .value_name("VALUE")
                .help("Brightness adjustment. Single value (-100 to +100) or comma-separated array (e.g., '0,20,-10')")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("contrast")
                .short('c')
                .long("contrast")
                .value_name("VALUE")
                .help("Contrast multiplier. Single value (0.1 to 3.0) or comma-separated array (e.g., '1.0,1.5,0.8')")
                .default_value("1.0"),
        )
        .arg(
            Arg::new("saturation")
                .short('s')
                .long("saturation")
                .value_name("VALUE")
                .help("Saturation multiplier. Single value (0.0 to 2.0) or comma-separated array (e.g., '1.0,1.8,0.5')")
                .default_value("1.0"),
        )
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .value_name("FORMAT")
                .help("Output format (jpg, png, tiff for images; mp4, mov, avi for video)")
                .default_value("mp4"),
        )
        .arg(
            Arg::new("fps")
                .short('r')
                .long("fps")
                .value_name("RATE")
                .help("Frame rate for video output (frames per second)")
                .default_value("24"),
        )
        .arg(
            Arg::new("quality")
                .short('q')
                .long("quality")
                .value_name("CRF")
                .help("Video quality (CRF: 0-51, lower = better quality, 18-28 recommended)")
                .default_value("20"),
        )
        .arg(
            Arg::new("resolution")
                .long("resolution")
                .value_name("WIDTHxHEIGHT")
                .help("Output video resolution (e.g., 1920x1080, 4K, HD). Default: original size"),
        )
        .arg(
            Arg::new("threads")
                .short('t')
                .long("threads")
                .value_name("NUM")
                .help("Number of threads to use for processing (default: auto-detect)")
                .default_value("0"),
        )
        .arg(
            Arg::new("start-frame")
                .long("start-frame")
                .value_name("INDEX")
                .help("Start frame index (0-based, inclusive). Default: 0 (first frame)"),
        )
        .arg(
            Arg::new("crop")
                .long("crop")
                .value_name("WIDTH:HEIGHT:X:Y")
                .help("Crop parameters in FFmpeg format (e.g., '1000:800:100:50' or '50%:50%:10%:10%')"),
        )
        .arg(
            Arg::new("end-frame")
                .long("end-frame")
                .value_name("INDEX")
                .help("End frame index (0-based, inclusive). Default: last frame"),
        )
        .get_matches();

    let input_dir = matches.get_one::<String>("input").unwrap();
    let output_dir = matches.get_one::<String>("output").unwrap();
    let format = matches.get_one::<String>("format").unwrap();
    let fps = matches
        .get_one::<String>("fps")
        .unwrap()
        .parse::<u32>()
        .map_err(|_| "Invalid fps value")?;
    let quality = matches
        .get_one::<String>("quality")
        .unwrap()
        .parse::<u32>()
        .map_err(|_| "Invalid quality value")?;
    let resolution = matches.get_one::<String>("resolution").map(|s| s.as_str());
    let threads = matches
        .get_one::<String>("threads")
        .unwrap()
        .parse::<usize>()
        .map_err(|_| "Invalid threads value")?;

    // Parse frame range arguments
    let start_frame = matches.get_one::<String>("start-frame")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| "Invalid start-frame value")?;
    
    let end_frame = matches.get_one::<String>("end-frame")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| "Invalid end-frame value")?;

    // Configure thread pool
    if threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|e| format!("Failed to configure thread pool: {}", e))?;
    }

    let adjustments = ImageAdjustments {
        exposure: parse_value_array(matches.get_one::<String>("exposure").unwrap())?,
        brightness: parse_value_array(matches.get_one::<String>("brightness").unwrap())?,
        contrast: parse_value_array(matches.get_one::<String>("contrast").unwrap())?,
        saturation: parse_value_array(matches.get_one::<String>("saturation").unwrap())?,
        crop: matches.get_one::<String>("crop").cloned(),
    };

    // Validate parameters
    validate_value_array(&adjustments.exposure, "Exposure", -3.0, 3.0)?;
    validate_value_array(&adjustments.brightness, "Brightness", -100.0, 100.0)?;
    validate_value_array(&adjustments.contrast, "Contrast", 0.1, 3.0)?;
    validate_value_array(&adjustments.saturation, "Saturation", 0.0, 2.0)?;
    
    if fps < 1 || fps > 120 {
        return Err("FPS must be between 1 and 120".into());
    }
    if quality > 51 {
        return Err("Quality (CRF) must be between 0 and 51".into());
    }

    // Validate frame range if provided
    if let (Some(start), Some(end)) = (start_frame, end_frame) {
        if start > end {
            return Err("Start frame must be less than or equal to end frame".into());
        }
    }

    let is_video_output = matches!(format.as_str(), "mp4" | "mov" | "avi");

    println!("{}", "Processing images with settings:".bold().cyan());
    print_value_array("Exposure", &adjustments.exposure, "EV");
    print_value_array("Brightness", &adjustments.brightness, "");
    print_value_array("Contrast", &adjustments.contrast, "x");
    print_value_array("Saturation", &adjustments.saturation, "x");
    
    // Print crop settings
    if let Some(ref crop_str) = adjustments.crop {
        println!("  {}: {}", "Crop".green(), crop_str);
    }
    
    if threads > 0 {
        println!("  {}: {} (manual)", "Threads".green(), threads);
    } else {
        println!("  {}: auto-detect ({} available)", "Threads".green(), rayon::current_num_threads());
    }
    if is_video_output {
        println!("  {}: {} video at {} fps (CRF {})", "Output".yellow(), format, fps, quality);
        if let Some(res) = resolution {
            println!("  {}: {}", "Resolution".yellow(), res);
        }
    } else {
        println!("  {}: {} images", "Output format".yellow(), format);
    }

    // Display frame range if specified
    if let Some(start) = start_frame {
        if let Some(end) = end_frame {
            println!("  {}: frames {} to {} ({} frames)", "Frame range".yellow(), start, end, end - start + 1);
        } else {
            println!("  {}: from frame {} to end", "Frame range".yellow(), start);
        }
    } else if let Some(end) = end_frame {
        println!("  {}: from start to frame {}", "Frame range".yellow(), end);
    }

    let start_time = Instant::now();

    if is_video_output {
        process_images_to_video(input_dir, output_dir, &adjustments, format, fps, quality, resolution, start_frame, end_frame, start_time)?;
    } else {
        process_images_to_images(input_dir, output_dir, &adjustments, format, start_frame, end_frame, start_time)?;
    }

    Ok(())
}

fn process_images_to_images(
    input_dir: &str,
    output_dir: &str,
    adjustments: &ImageAdjustments,
    output_format: &str,
    start_frame: Option<usize>,
    end_frame: Option<usize>,
    start_time: Instant,
) -> Result<(), Box<dyn Error>> {
    let input_path = Path::new(input_dir);
    let output_path = Path::new(output_dir);

    // Create output directory if it doesn't exist
    fs::create_dir_all(output_path)?;

    // Get list of image files
    let mut image_files: Vec<PathBuf> = fs::read_dir(input_path)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| is_image_file(path))
        .collect();

    image_files.sort();

    if image_files.is_empty() {
        return Err("No image files found in input directory".into());
    }

    // Apply frame range filtering
    let total_available_frames = image_files.len();
    let start_idx = start_frame.unwrap_or(0);
    let end_idx = end_frame.unwrap_or(total_available_frames - 1);
    
    // Validate frame range against available frames
    if start_idx >= total_available_frames {
        return Err(format!("Start frame {} is out of range (0-{})", start_idx, total_available_frames - 1).into());
    }
    if end_idx >= total_available_frames {
        return Err(format!("End frame {} is out of range (0-{})", end_idx, total_available_frames - 1).into());
    }
    
    // Filter to selected frame range
    let filtered_files: Vec<PathBuf> = image_files.into_iter()
        .skip(start_idx)
        .take(end_idx - start_idx + 1)
        .collect();
    
    let total_files = filtered_files.len();

    println!("{} {} image files", "Found".bold().blue(), total_available_frames);
    if start_idx > 0 || end_idx < total_available_frames - 1 {
        println!("{} {} frames ({} to {})", "Processing".bold().blue(), total_files, start_idx, end_idx);
    }

    // Create a counter for progress tracking
    let processed_count = Arc::new(AtomicUsize::new(0));
    let total_files = total_files;

    // Process images in parallel
    let results: Vec<Result<(), ProcessingError>> = filtered_files
        .par_iter()
        .enumerate()
        .map(|(i, image_path)| {
            let img = image::open(image_path)
                .map_err(|e| ProcessingError(format!("Failed to open image: {}", e)))?;
            
            // Calculate global frame index for proper interpolation
            let global_frame_index = start_idx + i;
            let processed_img = apply_adjustments(img, adjustments, global_frame_index, total_available_frames)
                .map_err(|e| ProcessingError(format!("Failed to apply adjustments: {}", e)))?;

            let output_filename = generate_output_filename(image_path, output_format);
            let output_file_path = output_path.join(output_filename);

            save_image(&processed_img, &output_file_path, output_format)
                .map_err(|e| ProcessingError(format!("Failed to save image: {}", e)))?;

            // Update progress
            let current = processed_count.fetch_add(1, Ordering::Relaxed) + 1;
            println!(
                "{} {}/{}: {}",
                "Processed".green(),
                current,
                total_files,
                image_path.file_name().unwrap().to_str().unwrap()
            );

            Ok(())
        })
        .collect();

    // Check for any errors
    for result in results {
        result.map_err(|e| Box::new(e) as Box<dyn Error>)?;
    }

    let processing_time = start_time.elapsed();
    println!("{}", "Image processing complete!".bold().green());
    println!("{}: {:.2?}", "Processing time".blue(), processing_time);
    Ok(())
}

fn process_images_to_video(
    input_dir: &str,
    output_dir: &str,
    adjustments: &ImageAdjustments,
    video_format: &str,
    fps: u32,
    quality: u32,
    resolution: Option<&str>,
    start_frame: Option<usize>,
    end_frame: Option<usize>,
    start_time: Instant,
) -> Result<(), Box<dyn Error>> {
    let input_path = Path::new(input_dir);
    let output_path = Path::new(output_dir);

    // Create output directory if it doesn't exist
    fs::create_dir_all(output_path)?;

    // Create temporary directory for processed images
    let temp_dir = output_path.join("temp_frames");
    fs::create_dir_all(&temp_dir)?;

    // Get list of image files
    let mut image_files: Vec<PathBuf> = fs::read_dir(input_path)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| is_image_file(path))
        .collect();

    image_files.sort();

    if image_files.is_empty() {
        return Err("No image files found in input directory".into());
    }

    println!("{} {} image files", "Found".bold().blue(), image_files.len());
    
    // Apply frame range filtering
    let total_available_frames = image_files.len();
    let start_idx = start_frame.unwrap_or(0);
    let end_idx = end_frame.unwrap_or(total_available_frames - 1);
    
    // Validate frame range against available frames
    if start_idx >= total_available_frames {
        return Err(format!("Start frame {} is out of range (0-{})", start_idx, total_available_frames - 1).into());
    }
    if end_idx >= total_available_frames {
        return Err(format!("End frame {} is out of range (0-{})", end_idx, total_available_frames - 1).into());
    }
    
    // Filter to selected frame range
    let filtered_files: Vec<PathBuf> = image_files.into_iter()
        .skip(start_idx)
        .take(end_idx - start_idx + 1)
        .collect();
    
    let total_files = filtered_files.len();
    
    if start_idx > 0 || end_idx < total_available_frames - 1 {
        println!("{} {} frames ({} to {})", "Processing".bold().blue(), total_files, start_idx, end_idx);
    }
    
    // Validate resolution proportions and get calculated output resolution
    // Note: Resolution validation is done on original images, but cropping happens during processing
    let calculated_resolution = validate_resolution_proportion(&filtered_files, resolution)?;
    
    // If cropping is specified, we need to adjust the resolution calculation
    // since the cropped images will have different dimensions
    let final_resolution = if let Some(ref crop_str) = adjustments.crop {
        // Get the first image to determine original dimensions
        if let Some(first_image_path) = filtered_files.first() {
            let img = image::open(first_image_path)?;
            let (original_width, original_height) = img.dimensions();
            
            // Parse crop parameters to get cropped dimensions
            let crop_params = parse_crop_string(crop_str)?;
            
            // Calculate actual cropped dimensions
            let x_offset = if crop_params.x < 0.0 {
                original_width as f32 + (crop_params.x / 100.0) * original_width as f32
            } else {
                crop_params.x
            };
            
            let y_offset = if crop_params.y < 0.0 {
                original_height as f32 + (crop_params.y / 100.0) * original_height as f32
            } else {
                crop_params.y
            };
            
            let crop_w = if crop_params.width <= 0.0 {
                original_width as f32 - x_offset
            } else if crop_params.width <= 100.0 && crop_params.width > 0.0 {
                (crop_params.width / 100.0) * original_width as f32
            } else {
                crop_params.width
            };
            
            let crop_h = if crop_params.height <= 0.0 {
                original_height as f32 - y_offset
            } else if crop_params.height <= 100.0 && crop_params.height > 0.0 {
                (crop_params.height / 100.0) * original_height as f32
            } else {
                crop_params.height
            };
            
            let cropped_width = crop_w as u32;
            let cropped_height = crop_h as u32;
            
            // Now validate resolution against cropped dimensions
            if let Some((target_width, target_height)) = calculated_resolution {
                let cropped_ratio = cropped_width as f32 / cropped_height as f32;
                let target_ratio = target_width as f32 / target_height as f32;
                
                // Calculate final output dimensions maintaining aspect ratio
                let final_width = if cropped_ratio > target_ratio {
                    // Cropped image is wider, fit to height
                    (target_height as f32 * cropped_ratio) as u32
                } else {
                    // Cropped image is taller, fit to width
                    target_width
                };
                
                let final_height = if cropped_ratio > target_ratio {
                    target_height
                } else {
                    (target_width as f32 / cropped_ratio) as u32
                };
                
                // Ensure even dimensions for H.264 compatibility
                let final_width = if final_width % 2 != 0 { final_width + 1 } else { final_width };
                let final_height = if final_height % 2 != 0 { final_height + 1 } else { final_height };
                
                println!("  {}: Cropped dimensions {}x{} -> Final output {}x{}", 
                    "Resolution".green(), cropped_width, cropped_height, final_width, final_height);
                
                Some((final_width, final_height))
            } else {
                calculated_resolution
            }
        } else {
            calculated_resolution
        }
    } else {
        calculated_resolution
    };
    
    println!("{}", "Processing images and creating video...".bold().cyan());

    // Calculate frame padding based on number of files
    let frame_padding = calculate_frame_padding(total_files);

    // Create a counter for progress tracking
    let processed_count = Arc::new(AtomicUsize::new(0));

    // Process images in parallel and save to temp directory
    let results: Vec<Result<(), ProcessingError>> = filtered_files
        .par_iter()
        .enumerate()
        .map(|(i, image_path)| {
            let img = image::open(image_path)
                .map_err(|e| ProcessingError(format!("Failed to open image: {}", e)))?;
            
            // Calculate global frame index for proper interpolation
            let global_frame_index = start_idx + i;
            let processed_img = apply_adjustments(img, adjustments, global_frame_index, total_available_frames)
                .map_err(|e| ProcessingError(format!("Failed to apply adjustments: {}", e)))?;

            // Save with dynamic sequential numbering for ffmpeg
            let temp_filename = format!("frame_{:0width$}.jpg", i + 1, width = frame_padding);
            let temp_file_path = temp_dir.join(temp_filename);

            save_image(&processed_img, &temp_file_path, "jpg")
                .map_err(|e| ProcessingError(format!("Failed to save image: {}", e)))?;

            // Update progress
            let current = processed_count.fetch_add(1, Ordering::Relaxed) + 1;
            print!("\r{} frame {}/{}", "Processing".yellow(), current, total_files);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();

            Ok(())
        })
        .collect();

    // Check for any errors
    for result in results {
        result.map_err(|e| Box::new(e) as Box<dyn Error>)?;
    }

    println!("\n{}", "Creating video with ffmpeg...".bold().cyan());

    // Generate output video filename
    let video_filename = format!("timelapse.{}", video_format);
    let video_output_path = output_path.join(video_filename);

    // Build ffmpeg command
    let mut ffmpeg_cmd = ProcessCommand::new("ffmpeg");
    ffmpeg_cmd
        .arg("-y") // Overwrite output file
        .arg("-framerate")
        .arg(fps.to_string())
        .arg("-i")
        .arg(temp_dir.join(format!("frame_%0{}d.jpg", frame_padding)))
        .arg("-c:v")
        .arg("libx264")
        .arg("-crf")
        .arg(quality.to_string())
        .arg("-pix_fmt")
        .arg("yuv420p");

    // Add resolution if specified
    if let Some((output_width, output_height)) = final_resolution {
        ffmpeg_cmd.arg("-vf").arg(format!("scale={}:{}", output_width, output_height));
    }

    ffmpeg_cmd.arg(&video_output_path);

    // Execute ffmpeg
    let output = ffmpeg_cmd.output()?;

    if !output.status.success() {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{}: {}", "FFmpeg failed".red(), error_msg).into());
    }

    // Clean up temporary files
    fs::remove_dir_all(&temp_dir)?;

    let processing_time = start_time.elapsed();
    println!("{}: {}", "Video created successfully".bold().green(), video_output_path.display());
    println!("{}: {:.2} seconds at {} fps", "Video duration".blue(), total_files as f32 / fps as f32, fps);
    println!("{}: {:.2?}", "Processing time".blue(), processing_time);

    Ok(())
}

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

fn apply_adjustments(img: DynamicImage, adjustments: &ImageAdjustments, frame_index: usize, total_frames: usize) -> Result<DynamicImage, ProcessingError> {
    let rgb_img = img.to_rgb8();
    let (width, height) = rgb_img.dimensions();
    
    // Get interpolated values for this frame
    let (exposure, brightness, contrast, saturation) = adjustments.get_values_at_frame(frame_index, total_frames);
    
    // Apply cropping first if specified
    let (start_x, start_y, end_x, end_y) = if let Some(ref crop_str) = adjustments.crop {
        let crop_params = parse_crop_string(crop_str)
            .map_err(|e| ProcessingError(format!("Failed to parse crop string: {}", e)))?;
        
        // Calculate crop coordinates
        let x_offset = if crop_params.x < 0.0 {
            // Negative values are percentages from the right
            width as f32 + (crop_params.x / 100.0) * width as f32
        } else {
            crop_params.x
        };
        
        let y_offset = if crop_params.y < 0.0 {
            // Negative values are percentages from the bottom
            height as f32 + (crop_params.y / 100.0) * height as f32
        } else {
            crop_params.y
        };
        
        let crop_w = if crop_params.width <= 0.0 {
            width as f32 - x_offset
        } else if crop_params.width <= 100.0 && crop_params.width > 0.0 {
            // Percentage
            (crop_params.width / 100.0) * width as f32
        } else {
            crop_params.width
        };
        
        let crop_h = if crop_params.height <= 0.0 {
            height as f32 - y_offset
        } else if crop_params.height <= 100.0 && crop_params.height > 0.0 {
            // Percentage
            (crop_params.height / 100.0) * height as f32
        } else {
            crop_params.height
        };
        
        let start_x = x_offset as u32;
        let start_y = y_offset as u32;
        let end_x = (start_x + crop_w as u32).min(width);
        let end_y = (start_y + crop_h as u32).min(height);
        
        (start_x, start_y, end_x, end_y)
    } else {
        // No cropping
        (0, 0, width, height)
    };
    
    let new_width = end_x - start_x;
    let new_height = end_y - start_y;
    
    let mut new_img = ImageBuffer::new(new_width, new_height);

    for (x, y, pixel) in rgb_img.enumerate_pixels() {
        // Skip pixels outside the crop area
        if x < start_x || x >= end_x || y < start_y || y >= end_y {
            continue;
        }
        
        let [r, g, b] = pixel.0;
        
        // Convert to float for processing
        let mut rf = r as f32 / 255.0;
        let mut gf = g as f32 / 255.0;
        let mut bf = b as f32 / 255.0;

        // Apply exposure (2^stops multiplier)
        if exposure != 0.0 {
            let exposure_multiplier = 2.0_f32.powf(exposure);
            rf *= exposure_multiplier;
            gf *= exposure_multiplier;
            bf *= exposure_multiplier;
        }

        // Apply brightness
        if brightness != 0.0 {
            let brightness_adjust = brightness / 100.0;
            rf += brightness_adjust;
            gf += brightness_adjust;
            bf += brightness_adjust;
        }

        // Apply contrast
        if contrast != 1.0 {
            rf = (rf - 0.5) * contrast + 0.5;
            gf = (gf - 0.5) * contrast + 0.5;
            bf = (bf - 0.5) * contrast + 0.5;
        }

        // Apply saturation
        if saturation != 1.0 {
            let gray = 0.299 * rf + 0.587 * gf + 0.114 * bf;
            rf = gray + (rf - gray) * saturation;
            gf = gray + (gf - gray) * saturation;
            bf = gray + (bf - gray) * saturation;
        }

        // Clamp values and convert back to u8
        let new_r = (rf.clamp(0.0, 1.0) * 255.0) as u8;
        let new_g = (gf.clamp(0.0, 1.0) * 255.0) as u8;
        let new_b = (bf.clamp(0.0, 1.0) * 255.0) as u8;

        // Map to new image coordinates
        let new_x = x - start_x;
        let new_y = y - start_y;
        new_img.put_pixel(new_x, new_y, Rgb([new_r, new_g, new_b]));
    }

    Ok(DynamicImage::ImageRgb8(new_img))
}

fn generate_output_filename(input_path: &Path, output_format: &str) -> String {
    let stem = input_path.file_stem().unwrap().to_str().unwrap();
    format!("{}_processed.{}", stem, output_format)
}

fn save_image(
    img: &DynamicImage,
    output_path: &Path,
    format: &str,
) -> Result<(), Box<dyn Error>> {
    match format.to_lowercase().as_str() {
        "jpg" | "jpeg" => {
            let rgb_img = img.to_rgb8();
            image::save_buffer(
                output_path,
                &rgb_img,
                rgb_img.width(),
                rgb_img.height(),
                image::ColorType::Rgb8,
            )?;
        }
        "png" => {
            img.save(output_path)?;
        }
        "tiff" | "tif" => {
            img.save(output_path)?;
        }
        _ => return Err(format!("Unsupported output format: {}", format).into()),
    }
    Ok(())
}