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
use rawloader::decode_file;

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
) -> Result<(), Box<dyn Error>> {
    if let Some(res) = target_resolution {
        // Get the first image to determine original dimensions
        if let Some(first_image_path) = image_files.first() {
            let img = load_image_with_raw_support(first_image_path, 2.0)?; // Default boost for validation
            let (original_width, original_height) = img.dimensions();
            
            // Parse target resolution
            let (target_width, target_height) = parse_resolution(res)?;
            
            // Calculate aspect ratios
            let original_ratio = original_width as f32 / original_height as f32;
            let target_ratio = target_width as f32 / target_height as f32;
            
            // Check if aspect ratios are significantly different (within 5% tolerance)
            let ratio_difference = (original_ratio - target_ratio).abs();
            let tolerance = 0.05;
            
            if ratio_difference > tolerance {
                println!(
                    "{}: Original aspect ratio ({:.2}:1) differs from target ({:.2}:1). This may cause distortion.",
                    "Warning".yellow(),
                    original_ratio,
                    target_ratio
                );
                println!("  Original dimensions: {}x{}", original_width, original_height);
                println!("  Target dimensions: {}x{}", target_width, target_height);
            } else {
                println!(
                    "{}: Aspect ratio validation passed ({:.2}:1)",
                    "Resolution".green(),
                    original_ratio
                );
            }
        }
    }
    Ok(())
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

fn load_image_with_raw_support(path: &Path, raw_boost: f32) -> Result<DynamicImage, Box<dyn Error>> {
    // First try to load with the standard image crate
    match image::open(path) {
        Ok(img) => Ok(img),
        Err(_) => {
            // If standard loading fails, try RAW format
            let raw_data = decode_file(path)
                .map_err(|e| format!("Failed to decode RAW file: {}", e))?;
            
            // Convert RAW data to DynamicImage
            let width = raw_data.width as u32;
            let height = raw_data.height as u32;
            

            
            // Create RGB buffer from RAW data
            let mut rgb_buffer = Vec::with_capacity((width * height * 3) as usize);
            
            // Access the data based on its type
            match raw_data.data {
                rawloader::RawImageData::Integer(data) => {
                    // Process integer data with proper color channel handling
                    process_raw_integer_data(&data, width, height, raw_boost, &mut rgb_buffer)?;
                }
                rawloader::RawImageData::Float(data) => {
                    // Process float data with proper color channel handling
                    process_raw_float_data(&data, width, height, raw_boost, &mut rgb_buffer)?;
                }
            }
            
            // Create ImageBuffer from RGB data
            let img_buffer = ImageBuffer::from_raw(width, height, rgb_buffer)
                .ok_or("Failed to create image buffer from RAW data")?;
            
            Ok(DynamicImage::ImageRgb8(img_buffer))
        }
    }
}

fn process_raw_integer_data(
    data: &[u16],
    width: u32,
    height: u32,
    raw_boost: f32,
    rgb_buffer: &mut Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    let width_usize = width as usize;
    let height_usize = height as usize;
    
    // Find min and max values for better normalization
    let min_val = data.iter().min().unwrap_or(&0);
    let max_val = data.iter().max().unwrap_or(&65535);
    let range = (max_val - min_val) as f32;
    

    
    // Simple approach: treat as grayscale with proper exposure
    for y in 0..height_usize {
        for x in 0..width_usize {
            let idx = y * width_usize + x;
            let pixel = data[idx];
            
            // Normalize to 0-1 range
            let normalized = if range > 0.0 {
                (pixel - min_val) as f32 / range
            } else {
                pixel as f32 / 65535.0
            };
            
            // Apply exposure boost and gamma correction
            let adjusted = if normalized > 0.0 {
                let gamma = 2.2;
                let corrected = (normalized * raw_boost).powf(1.0 / gamma);
                (corrected * 255.0).clamp(0.0, 255.0) as u8
            } else {
                0u8
            };
            
            // Use the same value for all channels (grayscale)
            // This avoids Bayer pattern issues
            rgb_buffer.push(adjusted);
            rgb_buffer.push(adjusted);
            rgb_buffer.push(adjusted);
        }
    }
    Ok(())
}

fn process_raw_float_data(
    data: &[f32],
    width: u32,
    height: u32,
    raw_boost: f32,
    rgb_buffer: &mut Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    let width_usize = width as usize;
    let height_usize = height as usize;
    
    // Find min and max values for better normalization
    let min_val = data.iter().fold(f32::INFINITY, |a, &b| a.min(b));
    let max_val = data.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let range = max_val - min_val;
    

    
    // Simple approach: treat as grayscale with proper exposure
    for y in 0..height_usize {
        for x in 0..width_usize {
            let idx = y * width_usize + x;
            let pixel = data[idx];
            
            // Normalize to 0-1 range
            let normalized = if range > 0.0 {
                (pixel - min_val) / range
            } else {
                pixel
            };
            
            // Apply exposure boost and gamma correction
            let adjusted = if normalized > 0.0 {
                let gamma = 2.2;
                let corrected = (normalized * raw_boost).powf(1.0 / gamma);
                (corrected * 255.0).clamp(0.0, 255.0) as u8
            } else {
                0u8
            };
            
            // Use the same value for all channels (grayscale)
            // This avoids Bayer pattern issues
            rgb_buffer.push(adjusted);
            rgb_buffer.push(adjusted);
            rgb_buffer.push(adjusted);
        }
    }
    Ok(())
}

// Note: Removed Bayer pattern interpolation functions as they were causing color issues
// Now using simple grayscale approach with proper exposure handling

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
            Arg::new("raw-boost")
                .long("raw-boost")
                .value_name("FACTOR")
                .help("Exposure boost factor for RAW files (default: 2.0)")
                .default_value("2.0"),
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

    let is_video_output = matches!(format.as_str(), "mp4" | "mov" | "avi");

    println!("{}", "Processing images with settings:".bold().cyan());
    print_value_array("Exposure", &adjustments.exposure, "EV");
    print_value_array("Brightness", &adjustments.brightness, "");
    print_value_array("Contrast", &adjustments.contrast, "x");
    print_value_array("Saturation", &adjustments.saturation, "x");
    
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

    let raw_boost = matches
        .get_one::<String>("raw-boost")
        .unwrap()
        .parse::<f32>()
        .map_err(|_| "Invalid raw-boost value")?;

    let start_time = Instant::now();

    if is_video_output {
        process_images_to_video(input_dir, output_dir, &adjustments, format, fps, quality, resolution, start_time, raw_boost)?;
    } else {
        process_images_to_images(input_dir, output_dir, &adjustments, format, start_time, raw_boost)?;
    }

    Ok(())
}

fn process_images_to_images(
    input_dir: &str,
    output_dir: &str,
    adjustments: &ImageAdjustments,
    output_format: &str,
    start_time: Instant,
    raw_boost: f32,
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

    println!("{} {} image files", "Found".bold().blue(), image_files.len());

    // Create a counter for progress tracking
    let processed_count = Arc::new(AtomicUsize::new(0));
    let total_files = image_files.len();

    // Process images in parallel
    let results: Vec<Result<(), ProcessingError>> = image_files
        .par_iter()
        .enumerate()
        .map(|(i, image_path)| {
            let img = load_image_with_raw_support(image_path, raw_boost)
                .map_err(|e| ProcessingError(format!("Failed to open image: {}", e)))?;
            let processed_img = apply_adjustments(img, adjustments, i, total_files);

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
    start_time: Instant,
    raw_boost: f32,
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
    
    // Validate resolution proportions
    validate_resolution_proportion(&image_files, resolution)?;
    
    println!("{}", "Processing images and creating video...".bold().cyan());

    // Calculate frame padding based on number of files
    let frame_padding = calculate_frame_padding(image_files.len());

    // Create a counter for progress tracking
    let processed_count = Arc::new(AtomicUsize::new(0));
    let total_files = image_files.len();

    // Process images in parallel and save to temp directory
    let results: Vec<Result<(), ProcessingError>> = image_files
        .par_iter()
        .enumerate()
        .map(|(i, image_path)| {
            let img = load_image_with_raw_support(image_path, raw_boost)
                .map_err(|e| ProcessingError(format!("Failed to open image: {}", e)))?;
            let processed_img = apply_adjustments(img, adjustments, i, total_files);

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
    if let Some(res) = resolution {
        let res_str = match res.to_lowercase().as_str() {
            "4k" => "3840x2160",
            "hd" | "1080p" => "1920x1080",
            "720p" => "1280x720",
            _ => res, // Use as-is for custom resolutions like "1920x1080"
        };
        ffmpeg_cmd.arg("-vf").arg(format!("scale={}", res_str));
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
    println!("{}: {:.2} seconds at {} fps", "Video duration".blue(), image_files.len() as f32 / fps as f32, fps);
    println!("{}: {:.2?}", "Processing time".blue(), processing_time);

    Ok(())
}

fn is_image_file(path: &Path) -> bool {
    if let Some(extension) = path.extension() {
        if let Some(ext_str) = extension.to_str() {
            matches!(
                ext_str.to_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "tiff" | "tif" | "bmp" | "webp" | 
                "raw" | "cr2" | "nef" | "arw" | "raf" | "dng" | "orf" | "rw2" | "pef" | "srw"
            )
        } else {
            false
        }
    } else {
        false
    }
}

fn apply_adjustments(img: DynamicImage, adjustments: &ImageAdjustments, frame_index: usize, total_frames: usize) -> DynamicImage {
    let rgb_img = img.to_rgb8();
    let (width, height) = rgb_img.dimensions();
    
    // Get interpolated values for this frame
    let (exposure, brightness, contrast, saturation) = adjustments.get_values_at_frame(frame_index, total_frames);
    
    let mut new_img = ImageBuffer::new(width, height);

    for (x, y, pixel) in rgb_img.enumerate_pixels() {
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

        new_img.put_pixel(x, y, Rgb([new_r, new_g, new_b]));
    }

    DynamicImage::ImageRgb8(new_img)
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