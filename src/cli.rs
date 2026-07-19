use std::path::Path;
use std::time::Instant;

use clap::{Arg, Command};
use colored::*;
use image::GenericImageView;

use crate::crop::validate_crop_and_offsets;
use crate::curve::{parse_value_array, validate_value_array};
use crate::error::{LapsifyError, Result};
use crate::export::images::process_images_to_images;
use crate::export::video::process_images_to_video;
use crate::render::ImageAdjustments;
use crate::source::list_images;

fn build_command() -> Command {
    Command::new("lapsify")
        .version(env!("CARGO_PKG_VERSION"))
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
            Arg::new("offset-x")
                .long("offset-x")
                .value_name("PIXELS")
                .help("X offset for crop window in pixels. Single value or comma-separated array. Examples: '10' (static), '0,20,0,-20' (panning), '0,5,-5,0' (stabilization)"),
        )
        .arg(
            Arg::new("offset-y")
                .long("offset-y")
                .value_name("PIXELS")
                .help("Y offset for crop window in pixels. Single value or comma-separated array. Examples: '-5' (static), '0,10,0,-10' (panning), '0,-3,3,0' (stabilization)"),
        )
        .arg(
            Arg::new("end-frame")
                .long("end-frame")
                .value_name("INDEX")
                .help("End frame index (0-based, inclusive). Default: last frame"),
        )
}

fn print_value_array(name: &str, values: &[f32], unit: &str) {
    if values.len() == 1 {
        println!("  {}: {}{}", name.green(), values[0], unit);
    } else {
        let values_str = values
            .iter()
            .map(|v| format!("{v}{unit}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {}: [{}]", name.green(), values_str);
    }
}

pub fn run() -> Result<()> {
    let matches = build_command().get_matches();

    let input_dir = matches.get_one::<String>("input").unwrap();
    let output_dir = matches.get_one::<String>("output").unwrap();
    let format = matches.get_one::<String>("format").unwrap();
    let fps = matches
        .get_one::<String>("fps")
        .unwrap()
        .parse::<u32>()
        .map_err(|_| LapsifyError::message("Invalid fps value"))?;
    let quality = matches
        .get_one::<String>("quality")
        .unwrap()
        .parse::<u32>()
        .map_err(|_| LapsifyError::message("Invalid quality value"))?;
    let resolution = matches.get_one::<String>("resolution").map(|s| s.as_str());
    let threads = matches
        .get_one::<String>("threads")
        .unwrap()
        .parse::<usize>()
        .map_err(|_| LapsifyError::message("Invalid threads value"))?;

    let start_frame = matches
        .get_one::<String>("start-frame")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| LapsifyError::message("Invalid start-frame value"))?;

    let end_frame = matches
        .get_one::<String>("end-frame")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| LapsifyError::message("Invalid end-frame value"))?;

    if threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|e| LapsifyError::message(format!("Failed to configure thread pool: {e}")))?;
    }

    let adjustments = ImageAdjustments {
        exposure: parse_value_array(matches.get_one::<String>("exposure").unwrap())?,
        brightness: parse_value_array(matches.get_one::<String>("brightness").unwrap())?,
        contrast: parse_value_array(matches.get_one::<String>("contrast").unwrap())?,
        saturation: parse_value_array(matches.get_one::<String>("saturation").unwrap())?,
        crop: matches.get_one::<String>("crop").cloned(),
        offset_x: parse_value_array(
            matches
                .get_one::<String>("offset-x")
                .unwrap_or(&"0.0".to_string()),
        )?,
        offset_y: parse_value_array(
            matches
                .get_one::<String>("offset-y")
                .unwrap_or(&"0.0".to_string()),
        )?,
    };

    validate_value_array(&adjustments.exposure, "Exposure", -3.0, 3.0)?;
    validate_value_array(&adjustments.brightness, "Brightness", -100.0, 100.0)?;
    validate_value_array(&adjustments.contrast, "Contrast", 0.1, 3.0)?;
    validate_value_array(&adjustments.saturation, "Saturation", 0.0, 2.0)?;

    if !(1..=120).contains(&fps) {
        return Err(LapsifyError::message("FPS must be between 1 and 120"));
    }
    if quality > 51 {
        return Err(LapsifyError::message(
            "Quality (CRF) must be between 0 and 51",
        ));
    }

    if let (Some(start), Some(end)) = (start_frame, end_frame) {
        if start > end {
            return Err(LapsifyError::message(
                "Start frame must be less than or equal to end frame",
            ));
        }
    }

    // Early validation of crop and offset boundaries against the first frame.
    if let Some(ref crop_str) = adjustments.crop {
        let image_files = list_images(Path::new(input_dir))?;
        let img = image::open(&image_files[0])?;
        let (width, height) = img.dimensions();
        validate_crop_and_offsets(
            crop_str,
            &adjustments.offset_x,
            &adjustments.offset_y,
            width,
            height,
        )?;
    }

    let is_video_output = matches!(format.as_str(), "mp4" | "mov" | "avi");

    println!("{}", "Processing images with settings:".bold().cyan());
    print_value_array("Exposure", &adjustments.exposure, "EV");
    print_value_array("Brightness", &adjustments.brightness, "");
    print_value_array("Contrast", &adjustments.contrast, "x");
    print_value_array("Saturation", &adjustments.saturation, "x");

    if let Some(ref crop_str) = adjustments.crop {
        println!("  {}: {}", "Crop".green(), crop_str);
        print_value_array("Offset X", &adjustments.offset_x, "px");
        print_value_array("Offset Y", &adjustments.offset_y, "px");
    }

    if threads > 0 {
        println!("  {}: {} (manual)", "Threads".green(), threads);
    } else {
        println!(
            "  {}: auto-detect ({} available)",
            "Threads".green(),
            rayon::current_num_threads()
        );
    }
    if is_video_output {
        println!(
            "  {}: {} video at {} fps (CRF {})",
            "Output".yellow(),
            format,
            fps,
            quality
        );
        if let Some(res) = resolution {
            println!("  {}: {}", "Resolution".yellow(), res);
        }
    } else {
        println!("  {}: {} images", "Output format".yellow(), format);
    }

    if let Some(start) = start_frame {
        if let Some(end) = end_frame {
            println!(
                "  {}: frames {} to {} ({} frames)",
                "Frame range".yellow(),
                start,
                end,
                end - start + 1
            );
        } else {
            println!("  {}: from frame {} to end", "Frame range".yellow(), start);
        }
    } else if let Some(end) = end_frame {
        println!("  {}: from start to frame {}", "Frame range".yellow(), end);
    }

    let start_time = Instant::now();

    if is_video_output {
        process_images_to_video(
            input_dir,
            output_dir,
            &adjustments,
            format,
            fps,
            quality,
            resolution,
            start_frame,
            end_frame,
            start_time,
        )?;
    } else {
        process_images_to_images(
            input_dir,
            output_dir,
            &adjustments,
            format,
            start_frame,
            end_frame,
            start_time,
        )?;
    }

    Ok(())
}
