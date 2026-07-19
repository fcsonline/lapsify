use std::path::Path;

use image::{DynamicImage, ImageBuffer, Rgb};

use crate::crop::{parse_crop_string, resolve_crop};
use crate::curve::interpolate_value;
use crate::error::{LapsifyError, Result};

#[derive(Debug, Clone)]
pub struct ImageAdjustments {
    pub exposure: Vec<f32>,
    pub brightness: Vec<f32>,
    pub contrast: Vec<f32>,
    pub saturation: Vec<f32>,
    pub crop: Option<String>,
    pub offset_x: Vec<f32>,
    pub offset_y: Vec<f32>,
}

impl Default for ImageAdjustments {
    fn default() -> Self {
        Self {
            exposure: vec![0.0],   // EV stops (+/- values)
            brightness: vec![0.0], // -100 to +100
            contrast: vec![1.0],   // 0.0 to 2.0 (1.0 = no change)
            saturation: vec![1.0], // 0.0 to 2.0 (1.0 = no change)
            crop: None,            // Crop string in format "width:height:x:y"
            offset_x: vec![0.0],   // X offset for crop window (pixels)
            offset_y: vec![0.0],   // Y offset for crop window (pixels)
        }
    }
}

impl ImageAdjustments {
    pub fn get_values_at_frame(
        &self,
        frame_index: usize,
        total_frames: usize,
    ) -> (f32, f32, f32, f32) {
        (
            interpolate_value(&self.exposure, frame_index, total_frames),
            interpolate_value(&self.brightness, frame_index, total_frames),
            interpolate_value(&self.contrast, frame_index, total_frames),
            interpolate_value(&self.saturation, frame_index, total_frames),
        )
    }
}

pub fn apply_adjustments(
    img: DynamicImage,
    adjustments: &ImageAdjustments,
    frame_index: usize,
    total_frames: usize,
) -> Result<DynamicImage> {
    let rgb_img = img.to_rgb8();
    let (width, height) = rgb_img.dimensions();

    let (exposure, brightness, contrast, saturation) =
        adjustments.get_values_at_frame(frame_index, total_frames);

    let offset_x = interpolate_value(&adjustments.offset_x, frame_index, total_frames);
    let offset_y = interpolate_value(&adjustments.offset_y, frame_index, total_frames);

    let (start_x, start_y, end_x, end_y) = if let Some(ref crop_str) = adjustments.crop {
        let crop_params = parse_crop_string(crop_str)?;
        let crop = resolve_crop(&crop_params, width, height);

        let start_x = (crop.x + offset_x) as u32;
        let start_y = (crop.y + offset_y) as u32;
        let end_x = (start_x + crop.width as u32).min(width);
        let end_y = (start_y + crop.height as u32).min(height);

        (start_x, start_y, end_x, end_y)
    } else {
        (0, 0, width, height)
    };

    let new_width = end_x - start_x;
    let new_height = end_y - start_y;

    let mut new_img = ImageBuffer::new(new_width, new_height);

    for (x, y, pixel) in rgb_img.enumerate_pixels() {
        if x < start_x || x >= end_x || y < start_y || y >= end_y {
            continue;
        }

        let [r, g, b] = pixel.0;

        let mut rf = r as f32 / 255.0;
        let mut gf = g as f32 / 255.0;
        let mut bf = b as f32 / 255.0;

        // Apply exposure (2^stops multiplier)
        if exposure != 0.0 {
            let exposure_multiplier = 2.0_f32.powf(exposure);
            rf *= exposure_multiplier;
            gf *= exposure_multiplier;
            bf *= exposure_multiplier;
        }

        if brightness != 0.0 {
            let brightness_adjust = brightness / 100.0;
            rf += brightness_adjust;
            gf += brightness_adjust;
            bf += brightness_adjust;
        }

        if contrast != 1.0 {
            rf = (rf - 0.5) * contrast + 0.5;
            gf = (gf - 0.5) * contrast + 0.5;
            bf = (bf - 0.5) * contrast + 0.5;
        }

        if saturation != 1.0 {
            let gray = 0.299 * rf + 0.587 * gf + 0.114 * bf;
            rf = gray + (rf - gray) * saturation;
            gf = gray + (gf - gray) * saturation;
            bf = gray + (bf - gray) * saturation;
        }

        let new_r = (rf.clamp(0.0, 1.0) * 255.0) as u8;
        let new_g = (gf.clamp(0.0, 1.0) * 255.0) as u8;
        let new_b = (bf.clamp(0.0, 1.0) * 255.0) as u8;

        let new_x = x - start_x;
        let new_y = y - start_y;
        new_img.put_pixel(new_x, new_y, Rgb([new_r, new_g, new_b]));
    }

    Ok(DynamicImage::ImageRgb8(new_img))
}

pub fn generate_output_filename(input_path: &Path, output_format: &str) -> String {
    let stem = input_path.file_stem().unwrap().to_str().unwrap();
    format!("{stem}_processed.{output_format}")
}

pub fn calculate_frame_padding(total_frames: usize) -> usize {
    if total_frames == 0 {
        1
    } else {
        total_frames.ilog10() as usize + 1
    }
}

pub fn save_image(img: &DynamicImage, output_path: &Path, format: &str) -> Result<()> {
    match format.to_lowercase().as_str() {
        "jpg" | "jpeg" => {
            let rgb_img = img.to_rgb8();
            image::save_buffer(
                output_path,
                &rgb_img,
                rgb_img.width(),
                rgb_img.height(),
                image::ExtendedColorType::Rgb8,
            )?;
        }
        "png" | "tiff" | "tif" => {
            img.save(output_path)?;
        }
        _ => {
            return Err(LapsifyError::message(format!(
                "Unsupported output format: {format}"
            )))
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_padding_widths() {
        assert_eq!(calculate_frame_padding(0), 1);
        assert_eq!(calculate_frame_padding(9), 1);
        assert_eq!(calculate_frame_padding(10), 2);
        assert_eq!(calculate_frame_padding(999), 3);
        assert_eq!(calculate_frame_padding(1000), 4);
    }

    #[test]
    fn output_filename_uses_stem_and_format() {
        assert_eq!(
            generate_output_filename(Path::new("/a/b/IMG_0001.CR3"), "jpg"),
            "IMG_0001_processed.jpg"
        );
    }

    #[test]
    fn adjustments_identity_keeps_pixels() {
        let mut img = ImageBuffer::new(4, 4);
        for (_, _, p) in img.enumerate_pixels_mut() {
            *p = Rgb([100u8, 150u8, 200u8]);
        }
        let img = DynamicImage::ImageRgb8(img);
        let out = apply_adjustments(img, &ImageAdjustments::default(), 0, 1).unwrap();
        let out = out.to_rgb8();
        assert_eq!(out.dimensions(), (4, 4));
        assert_eq!(out.get_pixel(0, 0).0, [100, 150, 200]);
    }

    #[test]
    fn exposure_one_stop_brightens() {
        let mut img = ImageBuffer::new(2, 2);
        for (_, _, p) in img.enumerate_pixels_mut() {
            *p = Rgb([64u8, 64u8, 64u8]);
        }
        let adjustments = ImageAdjustments {
            exposure: vec![1.0],
            ..Default::default()
        };
        let out = apply_adjustments(DynamicImage::ImageRgb8(img), &adjustments, 0, 1)
            .unwrap()
            .to_rgb8();
        assert_eq!(out.get_pixel(0, 0).0, [128, 128, 128]);
    }

    #[test]
    fn crop_reduces_dimensions() {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(200, 200, Rgb([10, 20, 30])));
        let adjustments = ImageAdjustments {
            crop: Some("120:110:10:20".to_string()),
            ..Default::default()
        };
        let out = apply_adjustments(img, &adjustments, 0, 1).unwrap();
        // 120/110 are <= 100? No: width 120 > 100 -> pixels; height 110 > 100 -> pixels.
        assert_eq!(out.to_rgb8().dimensions(), (120, 110));
    }
}
