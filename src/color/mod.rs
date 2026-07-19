//! Color pipeline.
//!
//! Order of operations per pixel:
//! 1. decode 8-bit sRGB to linear light
//! 2. linear-domain ops: exposure (2^EV)
//! 3. encode back to gamma-encoded sRGB
//! 4. display-domain ops: brightness offset, contrast around mid-gray
//! 5. saturation as a luma mix (Rec.709), cross-channel
//! 6. quantize to 8-bit
//!
//! Steps 1-4 are per-channel functions of the input byte, so they are baked
//! into a per-frame lookup table; only saturation runs per pixel.

pub mod lut;
pub mod transfer;

pub use lut::FrameColorOps;

use crate::project::Project;

/// Rec.709 luma coefficients.
pub const LUMA_R: f32 = 0.2126;
pub const LUMA_G: f32 = 0.7152;
pub const LUMA_B: f32 = 0.0722;

/// Color adjustment values resolved for a single frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorParams {
    /// Exposure in EV stops, applied in linear light.
    pub exposure: f32,
    /// Brightness offset in display space, -100..=100 maps to -1..=1.
    pub brightness: f32,
    /// Contrast multiplier around mid-gray in display space.
    pub contrast: f32,
    /// Saturation multiplier around Rec.709 luma.
    pub saturation: f32,
}

impl ColorParams {
    pub fn at_frame(project: &Project, frame: u32) -> Self {
        let color = &project.color;
        Self {
            exposure: color.exposure.sample(frame),
            brightness: color.brightness.sample(frame),
            contrast: color.contrast.sample(frame),
            saturation: color.saturation.sample(frame),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.exposure == 0.0
            && self.brightness == 0.0
            && self.contrast == 1.0
            && self.saturation == 1.0
    }

    /// The full tonal chain for one channel value (gamma-encoded 0..=1 in,
    /// gamma-encoded out). This is the definition of the pipeline; the LUT
    /// bakes it per frame.
    pub fn tonal_chain(&self, encoded: f32) -> f32 {
        // 1. decode to linear light
        let mut linear = transfer::srgb_to_linear(encoded);

        // 2. exposure in linear light
        if self.exposure != 0.0 {
            linear *= 2.0_f32.powf(self.exposure);
        }

        // 3. back to display space
        let mut v = transfer::linear_to_srgb(linear.clamp(0.0, 1.0));

        // 4. display-domain ops
        if self.brightness != 0.0 {
            v += self.brightness / 100.0;
        }
        if self.contrast != 1.0 {
            v = (v - 0.5) * self.contrast + 0.5;
        }

        v
    }

    /// Reference implementation of the whole pipeline for one pixel. Slow;
    /// used to verify the LUT path and as executable documentation.
    pub fn apply_reference(&self, rgb: [u8; 3]) -> [u8; 3] {
        let mut v = [0.0f32; 3];
        for (out, &channel) in v.iter_mut().zip(rgb.iter()) {
            *out = self.tonal_chain(channel as f32 / 255.0);
        }

        if self.saturation != 1.0 {
            let luma = LUMA_R * v[0] + LUMA_G * v[1] + LUMA_B * v[2];
            for channel in v.iter_mut() {
                *channel = luma + (*channel - luma) * self.saturation;
            }
        }

        [
            (v[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (v[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (v[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn identity_params_do_nothing() {
        let params = ColorParams {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
        };
        assert!(params.is_identity());
        assert_eq!(params.apply_reference([100, 150, 200]), [100, 150, 200]);
    }

    #[test]
    fn exposure_doubles_linear_light() {
        let params = ColorParams {
            exposure: 1.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
        };
        let out = params.tonal_chain(0.4);
        let linear_in = transfer::srgb_to_linear(0.4);
        let linear_out = transfer::srgb_to_linear(out);
        assert_relative_eq!(linear_out, 2.0 * linear_in, epsilon = 1e-5);
    }

    #[test]
    fn saturation_zero_is_grayscale() {
        let params = ColorParams {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 0.0,
        };
        let [r, g, b] = params.apply_reference([200, 30, 90]);
        assert_eq!(r, g);
        assert_eq!(g, b);
    }

    #[test]
    fn contrast_pivots_at_mid_gray() {
        let params = ColorParams {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 2.0,
            saturation: 1.0,
        };
        assert_relative_eq!(params.tonal_chain(0.5), 0.5, epsilon = 1e-6);
        assert!(params.tonal_chain(0.7) > 0.7);
        assert!(params.tonal_chain(0.3) < 0.3);
    }
}
