pub mod images;
pub mod video;

use std::path::PathBuf;

use colored::*;
use image::GenericImageView;

use crate::error::{LapsifyError, Result};

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

/// Validate the target resolution against the first image's aspect ratio and
/// compute the output dimensions (even-sized for H.264 compatibility).
pub fn validate_resolution_proportion(
    image_files: &[PathBuf],
    target_resolution: Option<&str>,
) -> Result<Option<(u32, u32)>> {
    let Some(res) = target_resolution else {
        return Ok(None);
    };
    let Some(first_image_path) = image_files.first() else {
        return Ok(None);
    };

    let img = image::open(first_image_path)?;
    let (original_width, original_height) = img.dimensions();

    let (target_width, target_height) = parse_resolution(res)?;

    let original_ratio = original_width as f32 / original_height as f32;
    let mut output_width = (target_height as f32 * original_ratio) as u32;
    if output_width % 2 != 0 {
        output_width += 1;
    }

    let mut output_height = target_height;
    if output_height % 2 != 0 {
        output_height += 1;
    }

    let target_ratio = target_width as f32 / target_height as f32;
    let ratio_difference = (original_ratio - target_ratio).abs();
    let tolerance = 0.05;

    if ratio_difference > tolerance {
        println!(
            "{}: Original aspect ratio ({:.2}:1) differs from target ({:.2}:1). This may cause distortion. {}: {}x{}",
            "Warning".yellow(),
            original_ratio,
            target_ratio,
            "Output resolution".yellow(),
            output_width,
            output_height
        );
    } else {
        println!(
            "{}: Aspect ratio validation passed ({:.2}:1). {}: {}x{}",
            "Resolution".green(),
            original_ratio,
            "Output resolution".green(),
            output_width,
            output_height
        );
    }

    Ok(Some((output_width, output_height)))
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
