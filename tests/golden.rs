//! Golden-image tests: render synthetic frames through a fixed project and
//! compare against committed expected images.
//!
//! Regenerate the expected images with: UPDATE_GOLDEN=1 cargo test --test golden

use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageBuffer, Rgb, RgbImage};
use lapsify::curve::Keyframe;
use lapsify::project::{ExportSettings, PROJECT_VERSION};
use lapsify::{render_frame, ColorGrade, CropRect, CropTrack, Curve, Project};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn gradient_image() -> DynamicImage {
    let img = ImageBuffer::from_fn(64, 48, |x, y| {
        Rgb([
            (x * 255 / 63) as u8,
            (y * 255 / 47) as u8,
            ((x + y) * 255 / 110) as u8,
        ])
    });
    DynamicImage::ImageRgb8(img)
}

fn color_bars_image() -> DynamicImage {
    let colors = [
        [255u8, 255, 255],
        [255, 255, 0],
        [0, 255, 255],
        [0, 255, 0],
        [255, 0, 255],
        [255, 0, 0],
        [0, 0, 255],
        [16, 16, 16],
    ];
    let img = ImageBuffer::from_fn(64, 48, |x, _| Rgb(colors[(x / 8) as usize % colors.len()]));
    DynamicImage::ImageRgb8(img)
}

fn noise_image() -> DynamicImage {
    // Deterministic LCG noise, no external randomness.
    let mut state: u32 = 0x12345678;
    let mut next = move || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state >> 24) as u8
    };
    let mut img = RgbImage::new(64, 48);
    for pixel in img.pixels_mut() {
        pixel.0 = [next(), next(), next()];
    }
    DynamicImage::ImageRgb8(img)
}

fn test_project() -> Project {
    Project {
        version: PROJECT_VERSION,
        input: PathBuf::from("unused"),
        frame_range: None,
        interpolation: Default::default(),
        color: ColorGrade {
            exposure: Curve::Keyframed(vec![Keyframe::new(0, -0.5), Keyframe::new(4, 0.8)]),
            brightness: Curve::Constant(5.0),
            contrast: Curve::Constant(1.2),
            saturation: Curve::Constant(1.3),
            ..ColorGrade::default()
        },
        crop: Some(CropTrack::from_rect(CropRect {
            x: 0.125,
            y: 0.125,
            width: 0.75,
            height: 0.75,
        })),
        export: {
            let mut export = ExportSettings::new(PathBuf::from("unused"));
            export.format = "png".to_string();
            export
        },
        analysis: None,
    }
}

fn check_golden(name: &str, img: DynamicImage, frame: u32) {
    let project = test_project();
    let rendered = render_frame(img, &project, frame).unwrap().to_rgb8();

    let expected_path = fixtures_dir().join(format!("{name}_frame{frame}.png"));

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(fixtures_dir()).unwrap();
        rendered.save(&expected_path).unwrap();
        return;
    }

    let expected = image::open(&expected_path)
        .unwrap_or_else(|_| {
            panic!(
                "Missing golden image {}. Run UPDATE_GOLDEN=1 cargo test --test golden",
                expected_path.display()
            )
        })
        .to_rgb8();

    assert_eq!(rendered.dimensions(), expected.dimensions(), "{name}: size");
    for (got, want) in rendered.pixels().zip(expected.pixels()) {
        for c in 0..3 {
            assert!(
                (got.0[c] as i16 - want.0[c] as i16).abs() <= 1,
                "{name} frame {frame}: pixel mismatch {:?} vs {:?}",
                got.0,
                want.0
            );
        }
    }
}

#[test]
fn golden_gradient() {
    check_golden("gradient", gradient_image(), 0);
    check_golden("gradient", gradient_image(), 4);
}

#[test]
fn golden_color_bars() {
    check_golden("bars", color_bars_image(), 2);
}

#[test]
fn golden_noise() {
    check_golden("noise", noise_image(), 3);
}
