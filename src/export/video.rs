use std::fs;
use std::path::Path;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use colored::*;
use image::GenericImageView;
use rayon::prelude::*;

use crate::crop::{parse_crop_string, resolve_crop};
use crate::error::{LapsifyError, Result};
use crate::export::validate_resolution_proportion;
use crate::render::{apply_adjustments, calculate_frame_padding, save_image, ImageAdjustments};
use crate::source::{list_images, select_frame_range};

#[allow(clippy::too_many_arguments)]
pub fn process_images_to_video(
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
) -> Result<()> {
    let input_path = Path::new(input_dir);
    let output_path = Path::new(output_dir);

    fs::create_dir_all(output_path).map_err(|e| LapsifyError::io(output_path, e))?;

    let temp_dir = output_path.join("temp_frames");
    fs::create_dir_all(&temp_dir).map_err(|e| LapsifyError::io(&temp_dir, e))?;

    let image_files = list_images(input_path)?;
    println!(
        "{} {} image files",
        "Found".bold().blue(),
        image_files.len()
    );

    let (filtered_files, start_idx, end_idx, total_available_frames) =
        select_frame_range(image_files, start_frame, end_frame)?;

    let total_files = filtered_files.len();

    if start_idx > 0 || end_idx < total_available_frames - 1 {
        println!(
            "{} {} frames ({} to {})",
            "Processing".bold().blue(),
            total_files,
            start_idx,
            end_idx
        );
    }

    let calculated_resolution = validate_resolution_proportion(&filtered_files, resolution)?;

    // Cropping changes frame dimensions, so recompute the output resolution
    // against the cropped size while preserving aspect ratio.
    let final_resolution = if let Some(ref crop_str) = adjustments.crop {
        if let (Some(first_image_path), Some((target_width, target_height))) =
            (filtered_files.first(), calculated_resolution)
        {
            let img = image::open(first_image_path)?;
            let (original_width, original_height) = img.dimensions();

            let crop_params = parse_crop_string(crop_str)?;
            let crop = resolve_crop(&crop_params, original_width, original_height);
            let cropped_width = crop.width as u32;
            let cropped_height = crop.height as u32;

            let cropped_ratio = cropped_width as f32 / cropped_height as f32;
            let target_ratio = target_width as f32 / target_height as f32;

            let final_width = if cropped_ratio > target_ratio {
                (target_height as f32 * cropped_ratio) as u32
            } else {
                target_width
            };
            let final_height = if cropped_ratio > target_ratio {
                target_height
            } else {
                (target_width as f32 / cropped_ratio) as u32
            };

            let final_width = final_width + final_width % 2;
            let final_height = final_height + final_height % 2;

            println!(
                "  {}: Cropped dimensions {}x{} -> Final output {}x{}",
                "Resolution".green(),
                cropped_width,
                cropped_height,
                final_width,
                final_height
            );

            Some((final_width, final_height))
        } else {
            calculated_resolution
        }
    } else {
        calculated_resolution
    };

    println!(
        "{}",
        "Processing images and creating video...".bold().cyan()
    );

    let frame_padding = calculate_frame_padding(total_files);

    let processed_count = Arc::new(AtomicUsize::new(0));

    let results: Vec<Result<()>> = filtered_files
        .par_iter()
        .enumerate()
        .map(|(i, image_path)| {
            let img = image::open(image_path)?;

            let global_frame_index = start_idx + i;
            let processed_img =
                apply_adjustments(img, adjustments, global_frame_index, total_available_frames)?;

            let temp_filename = format!("frame_{:0width$}.jpg", i + 1, width = frame_padding);
            let temp_file_path = temp_dir.join(temp_filename);

            save_image(&processed_img, &temp_file_path, "jpg")?;

            let current = processed_count.fetch_add(1, Ordering::Relaxed) + 1;
            print!(
                "\r{} frame {}/{}",
                "Processing".yellow(),
                current,
                total_files
            );
            std::io::Write::flush(&mut std::io::stdout()).unwrap();

            Ok(())
        })
        .collect();

    for result in results {
        result?;
    }
    println!();

    println!("\n{}", "Creating video with ffmpeg...".bold().cyan());

    let video_filename = format!("timelapse.{video_format}");
    let video_output_path = output_path.join(video_filename);

    let mut ffmpeg_cmd = ProcessCommand::new("ffmpeg");
    ffmpeg_cmd
        .arg("-y")
        .arg("-framerate")
        .arg(fps.to_string())
        .arg("-i")
        .arg(temp_dir.join(format!("frame_%0{frame_padding}d.jpg")))
        .arg("-c:v")
        .arg("libx264")
        .arg("-crf")
        .arg(quality.to_string())
        .arg("-pix_fmt")
        .arg("yuv420p");

    if let Some((output_width, output_height)) = final_resolution {
        ffmpeg_cmd
            .arg("-vf")
            .arg(format!("scale={output_width}:{output_height}"));
    }

    ffmpeg_cmd.arg(&video_output_path);

    let output = ffmpeg_cmd
        .output()
        .map_err(|e| LapsifyError::message(format!("Failed to run ffmpeg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LapsifyError::Ffmpeg {
            code: output.status.code(),
            stderr_tail: stderr
                .chars()
                .rev()
                .take(2000)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
        });
    }

    fs::remove_dir_all(&temp_dir).map_err(|e| LapsifyError::io(&temp_dir, e))?;

    let processing_time = start_time.elapsed();
    println!(
        "{}: {}",
        "Video created successfully".bold().green(),
        video_output_path.display()
    );
    println!(
        "{}: {:.2} seconds at {} fps",
        "Video duration".blue(),
        total_files as f32 / fps as f32,
        fps
    );
    println!("{}: {:.2?}", "Processing time".blue(), processing_time);

    Ok(())
}
