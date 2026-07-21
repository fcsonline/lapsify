use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rayon::prelude::*;

use crate::error::{LapsifyError, Result};
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::project::Project;
use crate::render::{generate_output_filename, render_frame, save_image};
use crate::source::{list_images, scan_dimensions, select_frame_range};

pub fn render_to_images(
    project: &Project,
    reporter: &ProgressReporter,
    start_time: Instant,
) -> Result<()> {
    let output_path = &project.export.output;
    fs::create_dir_all(output_path).map_err(|e| LapsifyError::io(output_path, e))?;

    let image_files = list_images(&project.input)?;
    let (src_w, src_h) = scan_dimensions(&image_files)?;

    let (start_frame, end_frame) = match project.frame_range {
        Some((start, end)) => (Some(start), Some(end)),
        None => (None, None),
    };
    let (filtered_files, start_idx, _, _) =
        select_frame_range(image_files, start_frame, end_frame)?;

    let total = filtered_files.len();
    let output_format = &project.export.format;
    let jpeg_quality = project.export.jpeg_quality;

    reporter.report(ProgressEvent::Start {
        total_frames: total,
        width: src_w,
        height: src_h,
    });

    let done = AtomicUsize::new(0);

    // Image files are independent, so no ordering is needed: write in place
    // from the rayon pool and count completions for progress.
    let results: Vec<Result<()>> = filtered_files
        .par_iter()
        .enumerate()
        .map(|(i, image_path)| {
            let img = crate::source::load_frame(image_path)?;

            // Global frame index keeps curve sampling aligned with the full sequence.
            let global_frame_index = (start_idx + i) as u32;
            let processed_img = render_frame(img, project, global_frame_index)?;

            let output_filename = generate_output_filename(image_path, output_format);
            let output_file_path = output_path.join(output_filename);

            save_image(
                &processed_img,
                &output_file_path,
                output_format,
                jpeg_quality,
            )?;

            let current = done.fetch_add(1, Ordering::Relaxed) + 1;
            reporter.report(ProgressEvent::Frame {
                index: i,
                done: current,
                total,
            });

            Ok(())
        })
        .collect();

    for result in results {
        result?;
    }

    reporter.report(ProgressEvent::Done {
        output: output_path.clone(),
        elapsed_ms: start_time.elapsed().as_millis() as u64,
    });

    Ok(())
}
