use std::fs;
use std::time::Instant;

use image::imageops::{self, FilterType};

use crate::error::{LapsifyError, Result};
use crate::export::ffmpeg::FfmpegSink;
use crate::export::render_ordered;
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::project::Project;
use crate::source::{list_images, scan_dimensions, select_frame_range};

pub fn render_to_video(
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

    // Raw video needs a fixed frame size. With a keyframed crop the window
    // size can vary per frame, so every frame is scaled to the size of the
    // first processed frame's window (that scaling is exactly a zoom).
    let (target_w, target_h) = match &project.crop {
        Some(track) => {
            let (_, _, w, h) = track.pixel_rect(start_idx as u32, src_w, src_h)?;
            (w, h)
        }
        None => (src_w, src_h),
    };

    reporter.report(ProgressEvent::Start {
        total_frames: total,
        width: target_w,
        height: target_h,
    });

    let (sink, output_file) = FfmpegSink::spawn(project, target_w, target_h)?;

    render_ordered(
        &filtered_files,
        project,
        start_idx,
        |frame| {
            if frame.dimensions() == (target_w, target_h) {
                frame
            } else {
                imageops::resize(&frame, target_w, target_h, FilterType::Lanczos3)
            }
        },
        Box::new(sink),
        reporter,
    )?;

    reporter.report(ProgressEvent::Done {
        output: output_file,
        elapsed_ms: start_time.elapsed().as_millis() as u64,
    });

    Ok(())
}
