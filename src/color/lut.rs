use image::RgbImage;

use super::{ColorParams, LUMA_B, LUMA_G, LUMA_R};

/// The per-frame color operation: the per-channel tonal chain baked into a
/// lookup table, plus the cross-channel saturation mix. Per-pixel cost stays
/// constant no matter how many tonal parameters the pipeline grows.
pub struct FrameColorOps {
    /// u8 in -> gamma-encoded f32 out, whole tonal chain baked in. One table
    /// per channel so channel-dependent ops (white balance) can slot in.
    lut: [[f32; 256]; 3],
    saturation: f32,
    identity: bool,
}

impl FrameColorOps {
    pub fn from_params(params: &ColorParams) -> Self {
        let mut channel = [0.0f32; 256];
        for (i, entry) in channel.iter_mut().enumerate() {
            *entry = params.tonal_chain(i as f32 / 255.0);
        }
        Self {
            lut: [channel; 3],
            saturation: params.saturation,
            identity: params.is_identity(),
        }
    }

    pub fn apply(&self, img: &mut RgbImage) {
        if self.identity {
            return;
        }

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
    use super::*;
    use image::Rgb;

    fn params(exposure: f32, brightness: f32, contrast: f32, saturation: f32) -> ColorParams {
        ColorParams {
            exposure,
            brightness,
            contrast,
            saturation,
        }
    }

    /// The LUT path must match the reference implementation for every input
    /// byte across a spread of parameter combinations.
    #[test]
    fn lut_matches_reference_implementation() {
        let cases = [
            params(0.0, 0.0, 1.0, 1.0),
            params(1.0, 0.0, 1.0, 1.0),
            params(-2.0, 0.0, 1.0, 1.0),
            params(0.5, 20.0, 1.0, 1.0),
            params(0.0, -30.0, 1.5, 1.0),
            params(0.0, 0.0, 0.5, 1.8),
            params(-0.7, 10.0, 1.3, 0.4),
        ];

        for case in &cases {
            let ops = FrameColorOps::from_params(case);
            for value in 0..=255u8 {
                for pixel in [[value, 30, 200], [value, value, value], [10, value, 90]] {
                    let mut img = RgbImage::from_pixel(1, 1, Rgb(pixel));
                    ops.apply(&mut img);
                    let expected = case.apply_reference(pixel);
                    let got = img.get_pixel(0, 0).0;
                    for c in 0..3 {
                        assert!(
                            (got[c] as i16 - expected[c] as i16).abs() <= 1,
                            "params {case:?} pixel {pixel:?}: lut {got:?} vs reference {expected:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn identity_leaves_image_untouched() {
        let ops = FrameColorOps::from_params(&params(0.0, 0.0, 1.0, 1.0));
        let mut img = RgbImage::from_pixel(2, 2, Rgb([12, 200, 99]));
        ops.apply(&mut img);
        assert_eq!(img.get_pixel(0, 0).0, [12, 200, 99]);
    }
}
