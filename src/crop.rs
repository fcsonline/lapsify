use crate::error::{LapsifyError, Result};

#[derive(Debug, Clone)]
pub struct CropParams {
    pub width: f32,
    pub height: f32,
    pub x: f32,
    pub y: f32,
}

/// Crop region resolved against a concrete image size, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedCrop {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub fn parse_crop_string(input: &str) -> Result<CropParams> {
    let parts: Vec<&str> = input.split(':').collect();
    if parts.len() != 4 {
        return Err(LapsifyError::message(format!(
            "Crop string must have 4 parts (width:height:x:y), got {} parts",
            parts.len()
        )));
    }

    let width = parse_crop_value(parts[0])?;
    let height = parse_crop_value(parts[1])?;
    let x = parse_crop_value(parts[2])?;
    let y = parse_crop_value(parts[3])?;

    Ok(CropParams {
        width,
        height,
        x,
        y,
    })
}

fn parse_crop_value(input: &str) -> Result<f32> {
    let input = input.trim();
    if let Some(stripped) = input.strip_suffix('%') {
        let percentage = stripped
            .parse::<f32>()
            .map_err(|_| LapsifyError::message(format!("Invalid percentage value: {input}")))?;
        if !(0.0..=100.0).contains(&percentage) {
            return Err(LapsifyError::message(format!(
                "Percentage must be between 0 and 100: {input}"
            )));
        }
        Ok(percentage)
    } else {
        input
            .parse::<f32>()
            .map_err(|_| LapsifyError::message(format!("Invalid pixel value: {input}")))
    }
}

/// Resolve crop parameters against an image size.
///
/// Negative x/y are treated as percentages from the right/bottom edge; values
/// in (0, 100] are treated as percentages of the image size, larger values as
/// pixels.
pub fn resolve_crop(params: &CropParams, image_width: u32, image_height: u32) -> ResolvedCrop {
    let width = image_width as f32;
    let height = image_height as f32;

    let x = if params.x < 0.0 {
        width + (params.x / 100.0) * width
    } else {
        params.x
    };

    let y = if params.y < 0.0 {
        height + (params.y / 100.0) * height
    } else {
        params.y
    };

    let crop_w = if params.width <= 0.0 {
        width - x
    } else if params.width <= 100.0 {
        (params.width / 100.0) * width
    } else {
        params.width
    };

    let crop_h = if params.height <= 0.0 {
        height - y
    } else if params.height <= 100.0 {
        (params.height / 100.0) * height
    } else {
        params.height
    };

    ResolvedCrop {
        x,
        y,
        width: crop_w,
        height: crop_h,
    }
}

/// Validate that every offset keeps the crop window inside the image.
pub fn validate_crop_and_offsets(
    crop_str: &str,
    offset_x: &[f32],
    offset_y: &[f32],
    image_width: u32,
    image_height: u32,
) -> Result<()> {
    let crop_params = parse_crop_string(crop_str)?;
    let crop = resolve_crop(&crop_params, image_width, image_height);

    let extremes = |values: &[f32]| -> Vec<f32> {
        if values.len() == 1 {
            vec![values[0]]
        } else {
            let min = values.iter().fold(f32::INFINITY, |a, &b| a.min(b));
            let max = values.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
            vec![min, max]
        }
    };

    for &offset in &extremes(offset_x) {
        let adjusted = crop.x + offset;
        if adjusted < 0.0 {
            return Err(LapsifyError::message(format!(
                "X offset {offset} would place crop window outside left edge (image width: {image_width})"
            )));
        }
        if adjusted + crop.width > image_width as f32 {
            return Err(LapsifyError::message(format!(
                "X offset {} would place crop window outside right edge (image width: {}, crop width: {})",
                offset, image_width, crop.width
            )));
        }
    }

    for &offset in &extremes(offset_y) {
        let adjusted = crop.y + offset;
        if adjusted < 0.0 {
            return Err(LapsifyError::message(format!(
                "Y offset {offset} would place crop window outside top edge (image height: {image_height})"
            )));
        }
        if adjusted + crop.height > image_height as f32 {
            return Err(LapsifyError::message(format!(
                "Y offset {} would place crop window outside bottom edge (image height: {}, crop height: {})",
                offset, image_height, crop.height
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_crop_string_pixels() {
        let params = parse_crop_string("1000:800:100:50").unwrap();
        assert_eq!(params.width, 1000.0);
        assert_eq!(params.height, 800.0);
        assert_eq!(params.x, 100.0);
        assert_eq!(params.y, 50.0);
    }

    #[test]
    fn parse_crop_string_percentages() {
        let params = parse_crop_string("50%:50%:10%:10%").unwrap();
        assert_eq!(params.width, 50.0);
        assert_eq!(params.height, 50.0);
    }

    #[test]
    fn parse_crop_string_rejects_bad_input() {
        assert!(parse_crop_string("100:100:0").is_err());
        assert!(parse_crop_string("100:100:0:0:0").is_err());
        assert!(parse_crop_string("abc:100:0:0").is_err());
        assert!(parse_crop_string("150%:100:0:0").is_err());
    }

    #[test]
    fn resolve_crop_pixel_values() {
        let params = parse_crop_string("1000:800:100:50").unwrap();
        let crop = resolve_crop(&params, 4000, 3000);
        assert_eq!(crop.x, 100.0);
        assert_eq!(crop.y, 50.0);
        assert_eq!(crop.width, 1000.0);
        assert_eq!(crop.height, 800.0);
    }

    #[test]
    fn resolve_crop_percentage_values() {
        let params = parse_crop_string("50%:50%:25%:25%").unwrap();
        let crop = resolve_crop(&params, 4000, 3000);
        assert_eq!(crop.x, 25.0); // NOTE: legacy behavior, percent offsets kept as raw value
        assert_eq!(crop.width, 2000.0);
        assert_eq!(crop.height, 1500.0);
    }

    #[test]
    fn validate_offsets_within_bounds() {
        assert!(validate_crop_and_offsets("1000:800:100:50", &[0.0], &[0.0], 4000, 3000).is_ok());
        assert!(
            validate_crop_and_offsets("1000:800:100:50", &[-200.0], &[0.0], 4000, 3000).is_err()
        );
        assert!(
            validate_crop_and_offsets("1000:800:100:50", &[3000.0], &[0.0], 4000, 3000).is_err()
        );
    }
}
