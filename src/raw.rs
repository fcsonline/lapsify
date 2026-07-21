//! Camera RAW decoding via `rawler` (pure Rust), behind the `raw` feature.
//!
//! RAW files decode through a neutral development pipeline (rescale,
//! demosaic, white balance, camera color calibration, crop, sRGB encode)
//! into a 16-bit RGB image that the rest of the engine treats like any
//! other source frame. Grading stays in lapsify's own pipeline, so RAW and
//! JPEG sequences behave identically downstream.

use std::path::Path;

use image::DynamicImage;
use rawler::decoders::{Orientation, RawDecodeParams};
use rawler::imgop::develop::RawDevelop;
use rawler::rawsource::RawSource;

use crate::error::{LapsifyError, Result};
use crate::exif::FrameExif;

/// Extensions rawler can decode. `dng` first: it is also the interchange
/// format most converters emit.
pub const RAW_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef",
    "iiq", "3fr", "erf", "kdc", "mos", "mrw", "dcr",
];

pub fn is_raw_extension(ext: &str) -> bool {
    RAW_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

fn open_source(path: &Path) -> Result<RawSource> {
    RawSource::new(path).map_err(|e| LapsifyError::io(path, e))
}

fn raw_error(path: &Path, context: &str, err: impl std::fmt::Display) -> LapsifyError {
    LapsifyError::message(format!("{context} {}: {err}", path.display()))
}

/// Decode and develop a RAW file into an sRGB image (16-bit channels),
/// with the camera orientation applied.
pub fn decode_raw(path: &Path) -> Result<DynamicImage> {
    let source = open_source(path)?;
    let decoder =
        rawler::get_decoder(&source).map_err(|e| raw_error(path, "Unsupported RAW file", e))?;
    let rawimage = decoder
        .raw_image(&source, &RawDecodeParams::default(), false)
        .map_err(|e| raw_error(path, "Failed to decode RAW file", e))?;

    let orientation = rawimage.orientation;
    let developed = RawDevelop::default()
        .develop_intermediate(&rawimage)
        .map_err(|e| raw_error(path, "Failed to develop RAW file", e))?;
    let img = developed
        .to_dynamic_image()
        .ok_or_else(|| raw_error(path, "Failed to develop RAW file", "empty developed image"))?;

    Ok(apply_orientation(img, orientation))
}

/// The dimensions `decode_raw` will produce, without decoding pixel data
/// (a "dummy" decode parses the container only).
pub fn raw_dimensions(path: &Path) -> Result<(u32, u32)> {
    let source = open_source(path)?;
    let decoder =
        rawler::get_decoder(&source).map_err(|e| raw_error(path, "Unsupported RAW file", e))?;
    let rawimage = decoder
        .raw_image(&source, &RawDecodeParams::default(), true)
        .map_err(|e| raw_error(path, "Failed to read RAW header of", e))?;

    // Development crops to the camera's default crop (or active area), so
    // the developed size differs from the sensor size.
    let (w, h) = rawimage
        .crop_area
        .or(rawimage.active_area)
        .map(|rect| (rect.d.w as u32, rect.d.h as u32))
        .unwrap_or((rawimage.width as u32, rawimage.height as u32));

    Ok(match rawimage.orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Transpose
        | Orientation::Transverse => (h, w),
        _ => (w, h),
    })
}

/// Exposure EXIF from the RAW container (used for formats the generic EXIF
/// reader cannot parse, like ISO-BMFF-based CR3).
pub fn raw_exif(path: &Path) -> Option<FrameExif> {
    let source = open_source(path).ok()?;
    let decoder = rawler::get_decoder(&source).ok()?;
    let metadata = decoder
        .raw_metadata(&source, &RawDecodeParams::default())
        .ok()?;
    let exif = metadata.exif;

    let rational = |r: rawler::formats::tiff::Rational| -> Option<f32> {
        (r.d != 0)
            .then(|| r.n as f32 / r.d as f32)
            .filter(|v| *v > 0.0)
    };

    let datetime_ms = exif
        .date_time_original
        .as_deref()
        .and_then(crate::exif::parse_exif_datetime_ms)
        .map(|ms| {
            let subsec = exif
                .sub_sec_time_original
                .as_deref()
                .and_then(|s| {
                    let digits: String = s.trim().chars().take(3).collect();
                    format!("{digits:0<3}").parse::<i64>().ok()
                })
                .unwrap_or(0);
            ms + subsec
        });

    Some(FrameExif {
        aperture: exif.fnumber.and_then(rational),
        shutter_s: exif.exposure_time.and_then(rational),
        iso: exif
            .iso_speed_ratings
            .map(|v| v as f32)
            .or(exif.iso_speed.map(|v| v as f32))
            .filter(|v| *v > 0.0),
        datetime_ms,
    })
}

fn apply_orientation(img: DynamicImage, orientation: Orientation) -> DynamicImage {
    match orientation {
        Orientation::Normal | Orientation::Unknown => img,
        Orientation::HorizontalFlip => img.fliph(),
        Orientation::Rotate180 => img.rotate180(),
        Orientation::VerticalFlip => img.flipv(),
        Orientation::Transpose => img.rotate90().fliph(),
        Orientation::Rotate90 => img.rotate90(),
        Orientation::Transverse => img.rotate270().fliph(),
        Orientation::Rotate270 => img.rotate270(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_raw_extensions() {
        assert!(is_raw_extension("dng"));
        assert!(is_raw_extension("CR2"));
        assert!(is_raw_extension("Arw"));
        assert!(!is_raw_extension("jpg"));
        assert!(!is_raw_extension("png"));
    }

    #[test]
    fn unreadable_raw_errors_cleanly() {
        assert!(decode_raw(Path::new("/nonexistent/file.dng")).is_err());
        assert!(raw_dimensions(Path::new("/nonexistent/file.dng")).is_err());
        assert!(raw_exif(Path::new("/nonexistent/file.dng")).is_none());
    }

    #[test]
    fn non_raw_content_is_rejected_not_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("fake.dng");
        std::fs::write(&fake, b"this is not a raw file").unwrap();
        assert!(decode_raw(&fake).is_err());
        assert!(raw_dimensions(&fake).is_err());
    }
}
