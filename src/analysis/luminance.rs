use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use image::RgbImage;
use rayon::prelude::*;
use xxhash_rust::xxh64::xxh64;

use crate::analysis::{now_unix, source_fingerprint, LumaSeries};
use crate::color::{transfer, ColorParams, FrameColorOps, LUMA_B, LUMA_G, LUMA_R};
use crate::crop::CropRect;
use crate::error::{LapsifyError, Result};
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::project::Project;

pub struct LuminanceOptions {
    /// Normalized source-image region to measure. None = full frame.
    pub region: Option<CropRect>,
    /// Downscale the long edge to this size before measuring.
    pub measure_dim: u32,
    /// Measure developed frames (all grading applied) instead of source.
    pub developed: bool,
}

impl Default for LuminanceOptions {
    fn default() -> Self {
        Self {
            region: None,
            measure_dim: 256,
            developed: false,
        }
    }
}

/// Measure per-frame mean linear luminance across the whole sequence.
///
/// Frames are downscaled to `measure_dim` first (cached under
/// `<input>/.lapsify/thumbs/`), and the developed path applies the color
/// pipeline only — the crop never changes pixel values, and the region is
/// defined in source-image space.
pub fn measure_luminance(
    project: &Project,
    image_files: &[PathBuf],
    opts: &LuminanceOptions,
    reporter: &ProgressReporter,
) -> Result<LumaSeries> {
    let fingerprint = source_fingerprint(image_files)?;
    let cache_dir = project.input.join(".lapsify").join("thumbs");
    let cache_usable = std::fs::create_dir_all(&cache_dir).is_ok();

    let total = image_files.len();
    let done = AtomicUsize::new(0);

    let values: Vec<f32> = image_files
        .par_iter()
        .enumerate()
        .map(|(frame, path)| -> Result<f32> {
            let mut thumb = load_thumbnail(
                path,
                opts.measure_dim,
                cache_usable.then_some(cache_dir.as_path()),
            )?;

            if opts.developed {
                let params = ColorParams::at_frame(project, frame as u32);
                FrameColorOps::from_params(&params).apply(&mut thumb);
            }

            let value = mean_linear_luma(&thumb, opts.region);

            let current = done.fetch_add(1, Ordering::Relaxed) + 1;
            reporter.report(ProgressEvent::Luma {
                frame,
                value,
                done: current,
                total,
            });

            Ok(value)
        })
        .collect::<Result<Vec<f32>>>()?;

    Ok(LumaSeries {
        values,
        region: opts.region,
        measure_dim: opts.measure_dim,
        computed_at_unix: now_unix(),
        source_fingerprint: fingerprint,
    })
}

/// Load a downscaled frame, using the on-disk thumbnail cache when possible.
fn load_thumbnail(path: &Path, measure_dim: u32, cache_dir: Option<&Path>) -> Result<RgbImage> {
    let cache_path = cache_dir.map(|dir| {
        let meta = std::fs::metadata(path);
        let (len, mtime) = meta
            .map(|m| {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                (m.len(), mtime)
            })
            .unwrap_or((0, 0));
        let key = xxh64(
            format!("{}|{len}|{mtime}|{measure_dim}", path.display()).as_bytes(),
            0,
        );
        dir.join(format!("{key:016x}.png"))
    });

    if let Some(ref cached) = cache_path {
        if let Ok(img) = image::open(cached) {
            return Ok(img.into_rgb8());
        }
    }

    let img = crate::source::load_frame(path)?;
    let thumb = if img.width() > measure_dim || img.height() > measure_dim {
        img.thumbnail(measure_dim, measure_dim)
    } else {
        img
    }
    .into_rgb8();

    if let Some(cached) = cache_path {
        let _ = thumb.save(&cached); // best-effort cache
    }

    Ok(thumb)
}

/// Mean linear Rec.709 luminance over a normalized region of the image.
fn mean_linear_luma(img: &RgbImage, region: Option<CropRect>) -> f32 {
    let (w, h) = img.dimensions();
    let (x0, y0, x1, y1) = match region {
        Some(r) => (
            ((r.x * w as f32) as u32).min(w.saturating_sub(1)),
            ((r.y * h as f32) as u32).min(h.saturating_sub(1)),
            (((r.x + r.width) * w as f32).ceil() as u32).clamp(1, w),
            (((r.y + r.height) * h as f32).ceil() as u32).clamp(1, h),
        ),
        None => (0, 0, w, h),
    };

    let decode = transfer::srgb_decode_table();
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let [r, g, b] = img.get_pixel(x, y).0;
            let luma = LUMA_R * decode[r as usize]
                + LUMA_G * decode[g as usize]
                + LUMA_B * decode[b as usize];
            sum += luma as f64;
            count += 1;
        }
    }

    if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    }
}

/// Parse a "X,Y,W,H" normalized region string.
pub fn parse_region(input: &str) -> Result<CropRect> {
    let parts: Vec<f32> = input
        .split(',')
        .map(|s| s.trim().parse::<f32>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| {
            LapsifyError::message(format!(
                "Invalid region '{input}' (expected X,Y,W,H as 0..1 fractions)"
            ))
        })?;
    if parts.len() != 4 {
        return Err(LapsifyError::message(
            "Region must have 4 comma-separated values (X,Y,W,H)",
        ));
    }
    let rect = CropRect {
        x: parts[0],
        y: parts[1],
        width: parts[2],
        height: parts[3],
    };
    let ok = (0.0..=1.0).contains(&rect.x)
        && (0.0..=1.0).contains(&rect.y)
        && rect.width > 0.0
        && rect.height > 0.0
        && rect.x + rect.width <= 1.0 + 1e-6
        && rect.y + rect.height <= 1.0 + 1e-6;
    if !ok {
        return Err(LapsifyError::message(
            "Region must lie within the image (normalized 0..1 coordinates)",
        ));
    }
    Ok(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use image::Rgb;

    #[test]
    fn mean_luma_of_uniform_gray() {
        // sRGB 188 is ~0.5 linear.
        let img = RgbImage::from_pixel(8, 8, Rgb([188, 188, 188]));
        let luma = mean_linear_luma(&img, None);
        assert_relative_eq!(luma, 0.5, epsilon = 0.01);
    }

    #[test]
    fn region_restricts_measurement() {
        // Left half black, right half white.
        let mut img = RgbImage::from_pixel(10, 10, Rgb([0, 0, 0]));
        for y in 0..10 {
            for x in 5..10 {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        let left = mean_linear_luma(
            &img,
            Some(CropRect {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }),
        );
        let right = mean_linear_luma(
            &img,
            Some(CropRect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }),
        );
        assert_relative_eq!(left, 0.0, epsilon = 1e-6);
        assert_relative_eq!(right, 1.0, epsilon = 1e-6);
    }

    #[test]
    fn parse_region_validates() {
        assert!(parse_region("0.1,0.1,0.5,0.5").is_ok());
        assert!(parse_region("0.8,0.0,0.5,0.5").is_err());
        assert!(parse_region("0,0,0.5").is_err());
        assert!(parse_region("a,b,c,d").is_err());
    }
}
