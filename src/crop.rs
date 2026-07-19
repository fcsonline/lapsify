use serde::{Deserialize, Serialize};

use crate::curve::{Curve, Keyframe};
use crate::error::{LapsifyError, Result};

/// A dimension from the legacy crop string: bare numbers are pixels, a `%`
/// suffix means percent of the image size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dim {
    Px(f32),
    Percent(f32),
}

/// A crop rectangle in normalized source-image coordinates (0..=1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A crop window over time. Every channel is an independently keyframable
/// curve in normalized source-image coordinates, so pans and zooms fall out
/// of ordinary keyframing.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CropTrack {
    pub x: Curve,
    pub y: Curve,
    pub width: Curve,
    pub height: Curve,
}

impl CropTrack {
    pub fn from_rect(rect: CropRect) -> Self {
        Self {
            x: Curve::Constant(rect.x),
            y: Curve::Constant(rect.y),
            width: Curve::Constant(rect.width),
            height: Curve::Constant(rect.height),
        }
    }

    /// The normalized crop rectangle at a frame.
    pub fn rect_at(&self, frame: u32) -> CropRect {
        CropRect {
            x: self.x.sample(frame),
            y: self.y.sample(frame),
            width: self.width.sample(frame),
            height: self.height.sample(frame),
        }
    }

    /// The crop rectangle at a frame in pixels, clamped to the image.
    pub fn pixel_rect(&self, frame: u32, src_w: u32, src_h: u32) -> Result<(u32, u32, u32, u32)> {
        let rect = self.rect_at(frame);
        let x = (rect.x * src_w as f32)
            .round()
            .clamp(0.0, (src_w - 1) as f32) as u32;
        let y = (rect.y * src_h as f32)
            .round()
            .clamp(0.0, (src_h - 1) as f32) as u32;
        let w = (rect.width * src_w as f32).round() as u32;
        let h = (rect.height * src_h as f32).round() as u32;
        let w = w.min(src_w - x);
        let h = h.min(src_h - y);
        if w == 0 || h == 0 {
            return Err(LapsifyError::message(format!(
                "Crop window is empty at frame {frame} ({w}x{h} at {x},{y} in a {src_w}x{src_h} image)"
            )));
        }
        Ok((x, y, w, h))
    }

    /// Structural validation of the four channel curves.
    pub fn validate(&self) -> Result<()> {
        self.x.validate("crop.x")?;
        self.y.validate("crop.y")?;
        self.width.validate("crop.width")?;
        self.height.validate("crop.height")?;
        self.x.validate_range("crop.x", 0.0, 1.0)?;
        self.y.validate_range("crop.y", 0.0, 1.0)?;
        self.width.validate_range("crop.width", 0.0, 1.0)?;
        self.height.validate_range("crop.height", 0.0, 1.0)?;
        Ok(())
    }

    /// Exact validation: sample every frame and check the window stays inside
    /// the image. Cheap (a few curve samples per frame) and exact, unlike
    /// checking keyframe extremes across independently keyframed channels.
    pub fn validate_over(&self, total_frames: usize) -> Result<()> {
        const EPS: f32 = 1e-4;
        for frame in 0..total_frames as u32 {
            let rect = self.rect_at(frame);
            if rect.width <= 0.0 || rect.height <= 0.0 {
                return Err(LapsifyError::message(format!(
                    "Crop window has no area at frame {frame}"
                )));
            }
            if rect.x < -EPS || rect.x + rect.width > 1.0 + EPS {
                return Err(LapsifyError::message(format!(
                    "Crop window exceeds horizontal image bounds at frame {frame} (x={}, width={})",
                    rect.x, rect.width
                )));
            }
            if rect.y < -EPS || rect.y + rect.height > 1.0 + EPS {
                return Err(LapsifyError::message(format!(
                    "Crop window exceeds vertical image bounds at frame {frame} (y={}, height={})",
                    rect.y, rect.height
                )));
            }
        }
        Ok(())
    }
}

/// Parse a legacy "width:height:x:y" crop string. Bare numbers are pixels
/// (negative = from the right/bottom edge), `%` values are percentages.
pub fn parse_crop_dims(input: &str) -> Result<[Dim; 4]> {
    let parts: Vec<&str> = input.split(':').collect();
    if parts.len() != 4 {
        return Err(LapsifyError::message(format!(
            "Crop string must have 4 parts (width:height:x:y), got {} parts",
            parts.len()
        )));
    }

    let mut dims = [Dim::Px(0.0); 4];
    for (i, part) in parts.iter().enumerate() {
        dims[i] = parse_crop_value(part)?;
    }
    Ok(dims)
}

fn parse_crop_value(input: &str) -> Result<Dim> {
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
        Ok(Dim::Percent(percentage))
    } else {
        let pixels = input
            .parse::<f32>()
            .map_err(|_| LapsifyError::message(format!("Invalid pixel value: {input}")))?;
        Ok(Dim::Px(pixels))
    }
}

/// Convert legacy crop dims + pixel offset curves into a normalized track.
///
/// Semantics: width/height 0 means "to the edge"; negative x/y positions the
/// window's right/bottom edge that many pixels from the image's right/bottom
/// edge.
pub fn legacy_crop_to_track(
    dims: [Dim; 4],
    offset_x: &Curve,
    offset_y: &Curve,
    src_w: u32,
    src_h: u32,
) -> Result<CropTrack> {
    let src_w = src_w as f32;
    let src_h = src_h as f32;

    let to_px = |dim: Dim, size: f32| -> f32 {
        match dim {
            Dim::Px(v) => v,
            Dim::Percent(p) => p / 100.0 * size,
        }
    };

    let mut w = to_px(dims[0], src_w);
    let mut h = to_px(dims[1], src_h);
    let x_raw = to_px(dims[2], src_w);
    let y_raw = to_px(dims[3], src_h);

    if w <= 0.0 {
        w = src_w - x_raw.max(0.0);
    }
    if h <= 0.0 {
        h = src_h - y_raw.max(0.0);
    }

    let x = if x_raw < 0.0 {
        src_w + x_raw - w
    } else {
        x_raw
    };
    let y = if y_raw < 0.0 {
        src_h + y_raw - h
    } else {
        y_raw
    };

    if x < 0.0 || y < 0.0 || x + w > src_w || y + h > src_h {
        return Err(LapsifyError::message(format!(
            "Crop window {w}x{h} at {x},{y} does not fit a {src_w}x{src_h} image"
        )));
    }

    // Fold the pixel offset curves into the normalized position curves,
    // preserving keyframe placement and easing.
    let fold = |base: f32, offsets: &Curve, size: f32| -> Curve {
        match offsets {
            Curve::Constant(off) => Curve::Constant((base + off) / size),
            Curve::Keyframed(keyframes) => Curve::Keyframed(
                keyframes
                    .iter()
                    .map(|k| Keyframe {
                        frame: k.frame,
                        value: (base + k.value) / size,
                        easing: k.easing,
                    })
                    .collect(),
            ),
        }
    };

    Ok(CropTrack {
        x: fold(x, offset_x, src_w),
        y: fold(y, offset_y, src_h),
        width: Curve::Constant(w / src_w),
        height: Curve::Constant(h / src_h),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn parses_pixels_and_percentages_distinctly() {
        let dims = parse_crop_dims("600:50%:100:-50").unwrap();
        assert_eq!(dims[0], Dim::Px(600.0));
        assert_eq!(dims[1], Dim::Percent(50.0));
        assert_eq!(dims[2], Dim::Px(100.0));
        assert_eq!(dims[3], Dim::Px(-50.0));
    }

    #[test]
    fn small_bare_numbers_are_pixels_not_percent() {
        let dims = parse_crop_dims("50:50:0:0").unwrap();
        let track = legacy_crop_to_track(
            dims,
            &Curve::Constant(0.0),
            &Curve::Constant(0.0),
            1000,
            500,
        )
        .unwrap();
        let (x, y, w, h) = track.pixel_rect(0, 1000, 500).unwrap();
        assert_eq!((x, y, w, h), (0, 0, 50, 50));
    }

    #[test]
    fn percent_values_scale_with_image() {
        let dims = parse_crop_dims("50%:50%:25%:25%").unwrap();
        let track = legacy_crop_to_track(
            dims,
            &Curve::Constant(0.0),
            &Curve::Constant(0.0),
            4000,
            3000,
        )
        .unwrap();
        let (x, y, w, h) = track.pixel_rect(0, 4000, 3000).unwrap();
        assert_eq!((x, y, w, h), (1000, 750, 2000, 1500));
    }

    #[test]
    fn negative_pixels_anchor_to_far_edge() {
        let dims = parse_crop_dims("600:400:-100:-50").unwrap();
        let track = legacy_crop_to_track(
            dims,
            &Curve::Constant(0.0),
            &Curve::Constant(0.0),
            4000,
            3000,
        )
        .unwrap();
        let (x, y, w, h) = track.pixel_rect(0, 4000, 3000).unwrap();
        // Right edge 100px from image right: x = 4000 - 100 - 600.
        assert_eq!((x, y), (3300, 2550));
        assert_eq!((w, h), (600, 400));
    }

    #[test]
    fn zero_size_extends_to_edge() {
        let dims = parse_crop_dims("0:0:100:200").unwrap();
        let track = legacy_crop_to_track(
            dims,
            &Curve::Constant(0.0),
            &Curve::Constant(0.0),
            1000,
            800,
        )
        .unwrap();
        let (x, y, w, h) = track.pixel_rect(0, 1000, 800).unwrap();
        assert_eq!((x, y, w, h), (100, 200, 900, 600));
    }

    #[test]
    fn rejects_out_of_bounds_window() {
        let dims = parse_crop_dims("2000:400:100:50").unwrap();
        assert!(legacy_crop_to_track(
            dims,
            &Curve::Constant(0.0),
            &Curve::Constant(0.0),
            1000,
            800
        )
        .is_err());
    }

    #[test]
    fn rejects_negative_percent() {
        assert!(parse_crop_dims("50%:50%:-10%:0").is_err());
        assert!(parse_crop_dims("wat:50%:0:0").is_err());
        assert!(parse_crop_dims("50:50:0").is_err());
    }

    #[test]
    fn offset_curves_fold_into_position() {
        let dims = parse_crop_dims("500:500:100:100").unwrap();
        let offsets = Curve::Keyframed(vec![Keyframe::new(0, 0.0), Keyframe::new(10, 100.0)]);
        let track =
            legacy_crop_to_track(dims, &offsets, &Curve::Constant(0.0), 1000, 1000).unwrap();
        assert_relative_eq!(track.x.sample(0), 0.1);
        assert_relative_eq!(track.x.sample(10), 0.2);
        let (x, ..) = track.pixel_rect(10, 1000, 1000).unwrap();
        assert_eq!(x, 200);
    }

    #[test]
    fn keyframed_track_pans_smoothly() {
        let track = CropTrack {
            x: Curve::Keyframed(vec![Keyframe::new(0, 0.0), Keyframe::new(100, 0.5)]),
            y: Curve::Constant(0.0),
            width: Curve::Constant(0.5),
            height: Curve::Constant(0.5),
        };
        assert!(track.validate().is_ok());
        assert!(track.validate_over(101).is_ok());
        let mid = track.rect_at(50);
        assert!(mid.x > 0.0 && mid.x < 0.5);
    }

    #[test]
    fn validate_over_catches_out_of_bounds_frames() {
        let track = CropTrack {
            x: Curve::Keyframed(vec![Keyframe::new(0, 0.0), Keyframe::new(100, 0.8)]),
            y: Curve::Constant(0.0),
            width: Curve::Constant(0.5),
            height: Curve::Constant(0.5),
        };
        // x + width reaches 1.3 by frame 100.
        assert!(track.validate_over(101).is_err());
        assert!(track.validate_over(10).is_ok());
    }

    #[test]
    fn empty_pixel_rect_errors() {
        let track = CropTrack::from_rect(CropRect {
            x: 0.999,
            y: 0.0,
            width: 0.0001,
            height: 0.5,
        });
        assert!(track.pixel_rect(0, 100, 100).is_err());
    }
}
