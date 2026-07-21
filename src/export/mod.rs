pub mod ffmpeg;
pub mod images;
pub mod video;

use std::collections::BTreeMap;
use std::path::PathBuf;

use crossbeam_channel::{bounded, Receiver};
use image::RgbImage;
use rayon::prelude::*;

use crate::error::{LapsifyError, Result};
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::project::Project;
use crate::render::render_frame;

/// A consumer of rendered frames. Frames arrive strictly in order.
pub trait FrameSink: Send {
    fn write_frame(&mut self, index: usize, frame: &RgbImage) -> Result<()>;
    fn finish(self: Box<Self>) -> Result<()>;
}

pub fn parse_resolution(resolution: &str) -> Result<(u32, u32)> {
    let res_str = match resolution.to_lowercase().as_str() {
        "4k" => "3840x2160",
        "hd" | "1080p" => "1920x1080",
        "720p" => "1280x720",
        _ => resolution,
    };

    let parts: Vec<&str> = res_str.split('x').collect();
    if parts.len() != 2 {
        return Err(LapsifyError::message(format!(
            "Invalid resolution format: {resolution}"
        )));
    }

    let width = parts[0]
        .parse::<u32>()
        .map_err(|_| LapsifyError::message(format!("Invalid width in resolution: {}", parts[0])))?;
    let height = parts[1].parse::<u32>().map_err(|_| {
        LapsifyError::message(format!("Invalid height in resolution: {}", parts[1]))
    })?;

    Ok((width, height))
}

/// Render frames in parallel and deliver them to the sink strictly in order.
///
/// Rendering fans out over rayon; a bounded channel provides backpressure and
/// a dedicated writer thread reorders results (work-stealing keeps in-flight
/// indices close together, so the reorder buffer stays small).
pub fn render_ordered(
    files: &[PathBuf],
    project: &Project,
    start_idx: usize,
    prepare: impl Fn(RgbImage) -> RgbImage + Sync,
    sink: Box<dyn FrameSink>,
    reporter: &ProgressReporter,
) -> Result<()> {
    let total = files.len();
    let (tx, rx) = bounded::<(usize, RgbImage)>(2 * rayon::current_num_threads());

    std::thread::scope(|scope| {
        let writer = scope.spawn(move || deliver_ordered(rx, sink, reporter, total));

        let produced =
            files
                .par_iter()
                .enumerate()
                .try_for_each_with(tx, |tx, (i, path)| -> Result<()> {
                    let img = crate::source::load_frame(path)?;
                    let frame = render_frame(img, project, (start_idx + i) as u32)?;
                    let frame = prepare(frame.into_rgb8());
                    tx.send((i, frame))
                        .map_err(|_| LapsifyError::message("frame writer terminated early"))?;
                    Ok(())
                });

        let written = writer
            .join()
            .map_err(|_| LapsifyError::message("frame writer thread panicked"))?;

        // The writer error is the root cause when both sides fail (producers
        // only see a closed channel).
        written.and(produced)
    })
}

fn deliver_ordered(
    rx: Receiver<(usize, RgbImage)>,
    mut sink: Box<dyn FrameSink>,
    reporter: &ProgressReporter,
    total: usize,
) -> Result<()> {
    let mut pending: BTreeMap<usize, RgbImage> = BTreeMap::new();
    let mut next = 0usize;

    for (index, frame) in rx.iter() {
        pending.insert(index, frame);
        while let Some(ready) = pending.remove(&next) {
            sink.write_frame(next, &ready)?;
            next += 1;
            reporter.report(ProgressEvent::Frame {
                index: next - 1,
                done: next,
                total,
            });
        }
    }

    if next != total {
        // A producer failed; its error is reported on the rayon side.
        return Err(LapsifyError::message(format!(
            "only {next} of {total} frames were rendered"
        )));
    }

    sink.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resolution_presets_and_custom() {
        assert_eq!(parse_resolution("4K").unwrap(), (3840, 2160));
        assert_eq!(parse_resolution("hd").unwrap(), (1920, 1080));
        assert_eq!(parse_resolution("1080p").unwrap(), (1920, 1080));
        assert_eq!(parse_resolution("720p").unwrap(), (1280, 720));
        assert_eq!(parse_resolution("640x480").unwrap(), (640, 480));
        assert!(parse_resolution("bogus").is_err());
        assert!(parse_resolution("640x").is_err());
    }
}
