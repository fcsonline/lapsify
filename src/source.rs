use std::fs;
use std::path::{Path, PathBuf};

use image::DynamicImage;

use crate::error::{LapsifyError, Result};

pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "tiff" | "tif" | "bmp" | "webp"
            ) || is_raw_path_ext(ext)
        })
        .unwrap_or(false)
}

/// Whether the path points at a camera RAW file (always false without the
/// `raw` feature).
pub fn is_raw_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(is_raw_path_ext)
        .unwrap_or(false)
}

#[cfg(feature = "raw")]
fn is_raw_path_ext(ext: &str) -> bool {
    crate::raw::is_raw_extension(ext)
}

#[cfg(not(feature = "raw"))]
fn is_raw_path_ext(_ext: &str) -> bool {
    false
}

/// Load a source frame: camera RAW files go through the RAW development
/// pipeline, everything else decodes directly.
pub fn load_frame(path: &Path) -> Result<DynamicImage> {
    #[cfg(feature = "raw")]
    if is_raw_path(path) {
        return crate::raw::decode_raw(path);
    }
    Ok(image::open(path)?)
}

/// List image files in a directory, sorted by filename.
pub fn list_images(input_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut image_files: Vec<PathBuf> = fs::read_dir(input_dir)
        .map_err(|e| LapsifyError::io(input_dir, e))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| is_image_file(path))
        .collect();

    image_files.sort();

    if image_files.is_empty() {
        return Err(LapsifyError::message(
            "No image files found in input directory",
        ));
    }

    Ok(image_files)
}

/// Read the dimensions of every frame (header-only, no full decode) and
/// verify they all match. Returns the common size.
pub fn scan_dimensions(image_files: &[PathBuf]) -> Result<(u32, u32)> {
    let mut expected: Option<(u32, u32)> = None;
    for (index, path) in image_files.iter().enumerate() {
        let (w, h) = frame_dimensions(path)?;
        match expected {
            None => expected = Some((w, h)),
            Some((want_w, want_h)) => {
                if (w, h) != (want_w, want_h) {
                    return Err(LapsifyError::MixedFrameSizes {
                        index,
                        path: path.clone(),
                        got_w: w,
                        got_h: h,
                        want_w,
                        want_h,
                    });
                }
            }
        }
    }
    expected.ok_or_else(|| LapsifyError::message("No image files to scan"))
}

/// Dimensions a frame will decode to, without decoding pixel data.
fn frame_dimensions(path: &Path) -> Result<(u32, u32)> {
    #[cfg(feature = "raw")]
    if is_raw_path(path) {
        return crate::raw::raw_dimensions(path);
    }
    Ok(image::ImageReader::open(path)
        .map_err(|e| LapsifyError::io(path, e))?
        .into_dimensions()?)
}

/// Select an inclusive frame range from the full sorted file list.
pub fn select_frame_range(
    image_files: Vec<PathBuf>,
    start_frame: Option<usize>,
    end_frame: Option<usize>,
) -> Result<(Vec<PathBuf>, usize, usize, usize)> {
    let total_available = image_files.len();
    let start_idx = start_frame.unwrap_or(0);
    let end_idx = end_frame.unwrap_or(total_available - 1);

    if start_idx >= total_available {
        return Err(LapsifyError::message(format!(
            "Start frame {} is out of range (0-{})",
            start_idx,
            total_available - 1
        )));
    }
    if end_idx >= total_available {
        return Err(LapsifyError::message(format!(
            "End frame {} is out of range (0-{})",
            end_idx,
            total_available - 1
        )));
    }

    let filtered: Vec<PathBuf> = image_files
        .into_iter()
        .skip(start_idx)
        .take(end_idx - start_idx + 1)
        .collect();

    Ok((filtered, start_idx, end_idx, total_available))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_extensions() {
        assert!(is_image_file(Path::new("a.jpg")));
        assert!(is_image_file(Path::new("a.JPEG")));
        assert!(is_image_file(Path::new("a.png")));
        assert!(is_image_file(Path::new("a.webp")));
        assert!(!is_image_file(Path::new("a.txt")));
        assert!(!is_image_file(Path::new("a")));

        // Camera RAW formats decode when the `raw` feature is on.
        #[cfg(feature = "raw")]
        {
            assert!(is_image_file(Path::new("a.cr2")));
            assert!(is_image_file(Path::new("a.NEF")));
            assert!(is_image_file(Path::new("a.arw")));
            assert!(is_image_file(Path::new("a.dng")));
            assert!(is_raw_path(Path::new("a.dng")));
            assert!(!is_raw_path(Path::new("a.jpg")));
        }
        #[cfg(not(feature = "raw"))]
        {
            assert!(!is_image_file(Path::new("a.cr2")));
            assert!(!is_image_file(Path::new("a.dng")));
        }
    }

    #[test]
    fn frame_range_selection() {
        let files: Vec<PathBuf> = (0..10).map(|i| PathBuf::from(format!("{i}.jpg"))).collect();
        let (selected, start, end, total) =
            select_frame_range(files.clone(), Some(2), Some(5)).unwrap();
        assert_eq!(selected.len(), 4);
        assert_eq!(start, 2);
        assert_eq!(end, 5);
        assert_eq!(total, 10);

        let (all, ..) = select_frame_range(files.clone(), None, None).unwrap();
        assert_eq!(all.len(), 10);

        assert!(select_frame_range(files, Some(10), None).is_err());
    }
}
