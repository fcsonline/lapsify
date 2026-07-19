use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use colored::*;
use rayon::prelude::*;

use crate::error::{LapsifyError, Result};
use crate::render::{apply_adjustments, generate_output_filename, save_image, ImageAdjustments};
use crate::source::{list_images, select_frame_range};

pub fn process_images_to_images(
    input_dir: &str,
    output_dir: &str,
    adjustments: &ImageAdjustments,
    output_format: &str,
    start_frame: Option<usize>,
    end_frame: Option<usize>,
    start_time: Instant,
) -> Result<()> {
    let input_path = Path::new(input_dir);
    let output_path = Path::new(output_dir);

    fs::create_dir_all(output_path).map_err(|e| LapsifyError::io(output_path, e))?;

    let image_files = list_images(input_path)?;
    let (filtered_files, start_idx, end_idx, total_available_frames) =
        select_frame_range(image_files, start_frame, end_frame)?;

    let total_files = filtered_files.len();

    println!(
        "{} {} image files",
        "Found".bold().blue(),
        total_available_frames
    );
    if start_idx > 0 || end_idx < total_available_frames - 1 {
        println!(
            "{} {} frames ({} to {})",
            "Processing".bold().blue(),
            total_files,
            start_idx,
            end_idx
        );
    }

    let processed_count = Arc::new(AtomicUsize::new(0));

    let results: Vec<Result<()>> = filtered_files
        .par_iter()
        .enumerate()
        .map(|(i, image_path)| {
            let img = image::open(image_path)?;

            // Global frame index keeps interpolation aligned with the full sequence.
            let global_frame_index = start_idx + i;
            let processed_img =
                apply_adjustments(img, adjustments, global_frame_index, total_available_frames)?;

            let output_filename = generate_output_filename(image_path, output_format);
            let output_file_path = output_path.join(output_filename);

            save_image(&processed_img, &output_file_path, output_format)?;

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

    for result in results {
        result?;
    }

    let processing_time = start_time.elapsed();
    println!("{}", "Image processing complete!".bold().green());
    println!("{}: {:.2?}", "Processing time".blue(), processing_time);
    Ok(())
}
