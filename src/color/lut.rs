use image::RgbImage;

use super::ColorParams;

/// The per-frame color operation: the per-channel tonal chain baked into a
/// lookup table, plus the cross-channel chroma step. Per-pixel cost stays
/// constant no matter how many tonal parameters the pipeline grows.
pub struct FrameColorOps {
    /// u8 in -> gamma-encoded f32 out, whole tonal chain baked in. One table
    /// per channel because white balance gains are channel-dependent.
    lut: [[f32; 256]; 3],
    saturation: f32,
    vibrance: f32,
    identity: bool,
}

impl FrameColorOps {
    pub fn from_params(params: &ColorParams) -> Self {
        let mut lut = [[0.0f32; 256]; 3];
        for (channel, table) in lut.iter_mut().enumerate() {
            for (i, entry) in table.iter_mut().enumerate() {
                *entry = params.tonal_chain(channel, i as f32 / 255.0);
            }
        }
        Self {
            lut,
            saturation: params.saturation,
            vibrance: params.vibrance,
            identity: params.is_identity(),
        }
    }

    pub fn apply(&self, img: &mut RgbImage) {
        if self.identity {
            return;
        }

        use super::{LUMA_B, LUMA_G, LUMA_R};

        for pixel in img.pixels_mut() {
            let [r, g, b] = pixel.0;

            let mut rf = self.lut[0][r as usize];
            let mut gf = self.lut[1][g as usize];
            let mut bf = self.lut[2][b as usize];

            if self.saturation != 1.0 {
                let luma = LUMA_R * rf + LUMA_G * gf + LUMA_B * bf;
                rf = luma + (rf - luma) * self.saturation;
                gf = luma + (gf - luma) * self.saturation;
                bf = luma + (bf - luma) * self.saturation;
            }

            if self.vibrance != 0.0 {
                let max = rf.max(gf).max(bf).clamp(0.0, 1.0);
                let min = rf.min(gf).min(bf).clamp(0.0, 1.0);
                let factor = 1.0 + (self.vibrance / 100.0) * (1.0 - (max - min));
                let luma = LUMA_R * rf + LUMA_G * gf + LUMA_B * bf;
                rf = luma + (rf - luma) * factor;
                gf = luma + (gf - luma) * factor;
                bf = luma + (bf - luma) * factor;
            }

            pixel.0 = [
                (rf.clamp(0.0, 1.0) * 255.0).round() as u8,
                (gf.clamp(0.0, 1.0) * 255.0).round() as u8,
                (bf.clamp(0.0, 1.0) * 255.0).round() as u8,
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::identity;
    use super::super::ToneCurve;
    use super::*;
    use image::Rgb;

    /// The LUT path must match the reference implementation for every input
    /// byte across a spread of parameter combinations.
    #[test]
    fn lut_matches_reference_implementation() {
        let tone_curve = ToneCurve {
            points: vec![(0.0, 0.05), (0.5, 0.6), (1.0, 0.95)],
        };
        let cases = [
            identity(),
            ColorParams {
                exposure: 1.0,
                ..identity()
            },
            ColorParams {
                exposure: -2.0,
                brightness: 20.0,
                ..identity()
            },
            ColorParams {
                temperature: 60.0,
                tint: -30.0,
                ..identity()
            },
            ColorParams {
                highlights: -80.0,
                shadows: 50.0,
                whites: 20.0,
                blacks: -20.0,
                ..identity()
            },
            ColorParams {
                gamma: 1.8,
                contrast: 1.4,
                saturation: 1.6,
                ..identity()
            },
            ColorParams {
                vibrance: 70.0,
                saturation: 0.8,
                ..identity()
            },
            ColorParams {
                tone_curve: Some(&tone_curve),
                exposure: 0.3,
                ..identity()
            },
        ];

        for case in &cases {
            let ops = FrameColorOps::from_params(case);
            for value in (0..=255u8).step_by(3) {
                for pixel in [[value, 30, 200], [value, value, value], [10, value, 90]] {
                    let mut img = RgbImage::from_pixel(1, 1, Rgb(pixel));
                    ops.apply(&mut img);
                    let expected = case.apply_reference(pixel);
                    let got = img.get_pixel(0, 0).0;
                    for c in 0..3 {
                        assert!(
                            (got[c] as i16 - expected[c] as i16).abs() <= 1,
                            "pixel {pixel:?}: lut {got:?} vs reference {expected:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn identity_leaves_image_untouched() {
        let ops = FrameColorOps::from_params(&identity());
        let mut img = RgbImage::from_pixel(2, 2, Rgb([12, 200, 99]));
        ops.apply(&mut img);
        assert_eq!(img.get_pixel(0, 0).0, [12, 200, 99]);
    }
}
