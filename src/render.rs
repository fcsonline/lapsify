use std::path::Path;

use image::{imageops, DynamicImage, RgbImage};

use crate::error::{LapsifyError, Result};
use crate::project::Project;

/// Adjustment values resolved for a single frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameParams {
    pub exposure: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl FrameParams {
    pub fn at_frame(project: &Project, frame: u32) -> Self {
        let color = &project.color;
        Self {
            exposure: color.exposure.sample(frame),
            brightness: color.brightness.sample(frame),
            contrast: color.contrast.sample(frame),
            saturation: color.saturation.sample(frame),
        }
    }
}

pub fn render_frame(img: DynamicImage, project: &Project, frame: u32) -> Result<DynamicImage> {
    let params = FrameParams::at_frame(project, frame);
    let rgb_img = img.into_rgb8();
    let (width, height) = rgb_img.dimensions();

    // Crop first so color work only touches pixels that survive.
    let mut out: RgbImage = match &project.crop {
        Some(track) => {
            let (x, y, w, h) = track.pixel_rect(frame, width, height)?;
            imageops::crop_imm(&rgb_img, x, y, w, h).to_image()
        }
        None => rgb_img,
    };

    apply_color(&mut out, &params);

    Ok(DynamicImage::ImageRgb8(out))
}

fn apply_color(img: &mut RgbImage, params: &FrameParams) {
    let identity = params.exposure == 0.0
        && params.brightness == 0.0
        && params.contrast == 1.0
        && params.saturation == 1.0;
    if identity {
        return;
    }

    let exposure_multiplier = 2.0_f32.powf(params.exposure);
    let brightness_adjust = params.brightness / 100.0;

    for pixel in img.pixels_mut() {
        let [r, g, b] = pixel.0;

        let mut rf = r as f32 / 255.0;
        let mut gf = g as f32 / 255.0;
        let mut bf = b as f32 / 255.0;

        // Apply exposure (2^stops multiplier)
        if params.exposure != 0.0 {
            rf *= exposure_multiplier;
            gf *= exposure_multiplier;
            bf *= exposure_multiplier;
        }

        if params.brightness != 0.0 {
            rf += brightness_adjust;
            gf += brightness_adjust;
            bf += brightness_adjust;
        }

        if params.contrast != 1.0 {
            rf = (rf - 0.5) * params.contrast + 0.5;
            gf = (gf - 0.5) * params.contrast + 0.5;
            bf = (bf - 0.5) * params.contrast + 0.5;
        }

        if params.saturation != 1.0 {
            let gray = 0.299 * rf + 0.587 * gf + 0.114 * bf;
            rf = gray + (rf - gray) * params.saturation;
            gf = gray + (gf - gray) * params.saturation;
            bf = gray + (bf - gray) * params.saturation;
        }

        pixel.0 = [
            (rf.clamp(0.0, 1.0) * 255.0) as u8,
            (gf.clamp(0.0, 1.0) * 255.0) as u8,
            (bf.clamp(0.0, 1.0) * 255.0) as u8,
        ];
    }
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
    use crate::crop::{CropRect, CropTrack};
    use crate::curve::{Curve, Keyframe};
    use crate::project::{ColorGrade, ExportSettings, Project, PROJECT_VERSION};
    use image::{ImageBuffer, Rgb};
    use std::path::PathBuf;

    fn test_project() -> Project {
        Project {
            version: PROJECT_VERSION,
            input: PathBuf::from("frames"),
            frame_range: None,
            color: ColorGrade::default(),
            crop: None,
            export: ExportSettings {
                output: PathBuf::from("out"),
                format: "jpg".to_string(),
                fps: 24,
                quality: 20,
                resolution: None,
            },
        }
    }

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
    fn identity_render_keeps_pixels() {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(4, 4, Rgb([100, 150, 200])));
        let out = render_frame(img, &test_project(), 0).unwrap().to_rgb8();
        assert_eq!(out.dimensions(), (4, 4));
        assert_eq!(out.get_pixel(0, 0).0, [100, 150, 200]);
    }

    #[test]
    fn exposure_one_stop_brightens() {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(2, 2, Rgb([64, 64, 64])));
        let mut project = test_project();
        project.color.exposure = Curve::Constant(1.0);
        let out = render_frame(img, &project, 0).unwrap().to_rgb8();
        assert_eq!(out.get_pixel(0, 0).0, [128, 128, 128]);
    }

    #[test]
    fn keyframed_exposure_varies_per_frame() {
        let mut project = test_project();
        project.color.exposure =
            Curve::Keyframed(vec![Keyframe::new(0, 0.0), Keyframe::new(10, 1.0)]);

        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(2, 2, Rgb([64, 64, 64])));
        let at_start = render_frame(img.clone(), &project, 0).unwrap().to_rgb8();
        let at_end = render_frame(img, &project, 10).unwrap().to_rgb8();
        assert_eq!(at_start.get_pixel(0, 0).0, [64, 64, 64]);
        assert_eq!(at_end.get_pixel(0, 0).0, [128, 128, 128]);
    }

    #[test]
    fn crop_reduces_dimensions() {
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(200, 200, Rgb([10, 20, 30])));
        let mut project = test_project();
        project.crop = Some(CropTrack::from_rect(CropRect {
            x: 0.05,
            y: 0.10,
            width: 0.60,
            height: 0.55,
        }));
        let out = render_frame(img, &project, 0).unwrap();
        assert_eq!(out.to_rgb8().dimensions(), (120, 110));
    }

    #[test]
    fn keyframed_crop_pans_across_frames() {
        // 100x100 image, 50x50 window panning from left to right.
        let mut img = ImageBuffer::from_pixel(100, 100, Rgb([0u8, 0, 0]));
        for x in 50..100 {
            for y in 0..100 {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        let img = DynamicImage::ImageRgb8(img);

        let mut project = test_project();
        project.crop = Some(CropTrack {
            x: Curve::Keyframed(vec![Keyframe::new(0, 0.0), Keyframe::new(10, 0.5)]),
            y: Curve::Constant(0.0),
            width: Curve::Constant(0.5),
            height: Curve::Constant(0.5),
        });

        let left = render_frame(img.clone(), &project, 0).unwrap().to_rgb8();
        let right = render_frame(img, &project, 10).unwrap().to_rgb8();
        assert_eq!(left.dimensions(), (50, 50));
        assert_eq!(left.get_pixel(0, 0).0, [0, 0, 0]);
        assert_eq!(right.get_pixel(0, 0).0, [255, 255, 255]);
    }
}
