//! Color pipeline.
//!
//! Order of operations per pixel:
//! 1. decode 8-bit sRGB to linear light
//! 2. linear-domain ops: white balance gains (temperature/tint), exposure (2^EV)
//! 3. encode back to gamma-encoded sRGB
//! 4. display-domain tonal ops: highlights, shadows, whites, blacks,
//!    brightness offset, contrast around mid-gray, gamma, tone curve
//! 5. cross-channel: saturation and vibrance as Rec.709 luma mixes
//! 6. quantize to 8-bit
//!
//! Steps 1-4 are per-channel functions of the input byte, so they are baked
//! into a per-frame lookup table; only step 5 runs per pixel.

pub mod lut;
pub mod tone;
pub mod transfer;

pub use lut::FrameColorOps;
pub use tone::ToneCurve;

use crate::project::Project;

/// Rec.709 luma coefficients.
pub const LUMA_R: f32 = 0.2126;
pub const LUMA_G: f32 = 0.7152;
pub const LUMA_B: f32 = 0.0722;

/// Color adjustment values resolved for a single frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorParams<'a> {
    /// Exposure in EV stops, applied in linear light.
    pub exposure: f32,
    /// White balance temperature, -100 (cool) to +100 (warm), linear gains.
    pub temperature: f32,
    /// White balance tint, -100 (green) to +100 (magenta), linear gains.
    pub tint: f32,
    /// Brightness offset in display space, -100..=100 maps to -1..=1.
    pub brightness: f32,
    /// Contrast multiplier around mid-gray in display space.
    pub contrast: f32,
    /// Highlight recovery/boost, -100..=100 (affects tones above mid-gray).
    pub highlights: f32,
    /// Shadow lift/crush, -100..=100 (affects tones below mid-gray).
    pub shadows: f32,
    /// White point adjustment, -100..=100 (affects the very top of the range).
    pub whites: f32,
    /// Black point adjustment, -100..=100 (affects the very bottom).
    pub blacks: f32,
    /// Midtone gamma, 0.2..=5.0 (1.0 = neutral, higher brightens midtones).
    pub gamma: f32,
    /// Saturation multiplier around Rec.709 luma.
    pub saturation: f32,
    /// Vibrance, -100..=100: saturation boost weighted toward muted colors.
    pub vibrance: f32,
    /// Optional parametric tone curve in display space.
    pub tone_curve: Option<&'a ToneCurve>,
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl<'a> ColorParams<'a> {
    pub fn at_frame(project: &'a Project, frame: u32) -> Self {
        let color = &project.color;
        Self {
            exposure: color.exposure.sample(frame),
            temperature: color.temperature.sample(frame),
            tint: color.tint.sample(frame),
            brightness: color.brightness.sample(frame),
            contrast: color.contrast.sample(frame),
            highlights: color.highlights.sample(frame),
            shadows: color.shadows.sample(frame),
            whites: color.whites.sample(frame),
            blacks: color.blacks.sample(frame),
            gamma: color.gamma.sample(frame),
            saturation: color.saturation.sample(frame),
            vibrance: color.vibrance.sample(frame),
            tone_curve: color.tone_curve.as_ref(),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.exposure == 0.0
            && self.temperature == 0.0
            && self.tint == 0.0
            && self.brightness == 0.0
            && self.contrast == 1.0
            && self.highlights == 0.0
            && self.shadows == 0.0
            && self.whites == 0.0
            && self.blacks == 0.0
            && self.gamma == 1.0
            && self.saturation == 1.0
            && self.vibrance == 0.0
            && self.tone_curve.is_none()
    }

    /// Linear-light gain for one channel from white balance, in stops.
    /// Warmer temperature boosts red and cuts blue; positive tint pushes
    /// toward magenta by cutting green.
    fn wb_gain(&self, channel: usize) -> f32 {
        let t = self.temperature / 100.0;
        let g = self.tint / 100.0;
        match channel {
            0 => 2.0_f32.powf(0.4 * t),
            1 => 2.0_f32.powf(-0.3 * g),
            2 => 2.0_f32.powf(-0.4 * t),
            _ => 1.0,
        }
    }

    /// The full tonal chain for one channel value (gamma-encoded 0..=1 in,
    /// gamma-encoded out). This is the definition of the pipeline; the LUT
    /// bakes it per frame and channel.
    pub fn tonal_chain(&self, channel: usize, encoded: f32) -> f32 {
        // 1. decode to linear light
        let mut linear = transfer::srgb_to_linear(encoded);

        // 2. linear-domain: white balance, then exposure
        linear *= self.wb_gain(channel);
        if self.exposure != 0.0 {
            linear *= 2.0_f32.powf(self.exposure);
        }

        // 3. back to display space
        let mut v = transfer::linear_to_srgb(linear.clamp(0.0, 1.0));

        // 4. display-domain tonal ops. Region weights are smooth so
        //    adjustments blend into neighboring tones without banding.
        if self.highlights != 0.0 {
            v += (self.highlights / 100.0) * 0.25 * smoothstep(0.5, 1.0, v);
        }
        if self.shadows != 0.0 {
            v += (self.shadows / 100.0) * 0.25 * (1.0 - smoothstep(0.0, 0.5, v));
        }
        if self.whites != 0.0 {
            v += (self.whites / 100.0) * 0.15 * smoothstep(0.7, 1.0, v);
        }
        if self.blacks != 0.0 {
            v += (self.blacks / 100.0) * 0.15 * (1.0 - smoothstep(0.0, 0.3, v));
        }

        if self.brightness != 0.0 {
            v += self.brightness / 100.0;
        }
        if self.contrast != 1.0 {
            v = (v - 0.5) * self.contrast + 0.5;
        }
        if self.gamma != 1.0 {
            v = v.clamp(0.0, 1.0).powf(1.0 / self.gamma);
        }
        if let Some(curve) = self.tone_curve {
            v = curve.sample(v.clamp(0.0, 1.0));
        }

        v
    }

    /// Cross-channel step: saturation, then vibrance (a saturation boost
    /// weighted toward muted colors).
    pub fn apply_chroma(&self, v: &mut [f32; 3]) {
        if self.saturation != 1.0 {
            let luma = LUMA_R * v[0] + LUMA_G * v[1] + LUMA_B * v[2];
            for channel in v.iter_mut() {
                *channel = luma + (*channel - luma) * self.saturation;
            }
        }
        if self.vibrance != 0.0 {
            let max = v[0].max(v[1]).max(v[2]).clamp(0.0, 1.0);
            let min = v[0].min(v[1]).min(v[2]).clamp(0.0, 1.0);
            let sat = max - min;
            let factor = 1.0 + (self.vibrance / 100.0) * (1.0 - sat);
            let luma = LUMA_R * v[0] + LUMA_G * v[1] + LUMA_B * v[2];
            for channel in v.iter_mut() {
                *channel = luma + (*channel - luma) * factor;
            }
        }
    }

    /// Reference implementation of the whole pipeline for one pixel. Slow;
    /// used to verify the LUT path and as executable documentation.
    pub fn apply_reference(&self, rgb: [u8; 3]) -> [u8; 3] {
        let mut v = [0.0f32; 3];
        for (channel, (out, &byte)) in v.iter_mut().zip(rgb.iter()).enumerate() {
            *out = self.tonal_chain(channel, byte as f32 / 255.0);
        }

        self.apply_chroma(&mut v);

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

    pub(crate) fn identity() -> ColorParams<'static> {
        ColorParams {
            exposure: 0.0,
            temperature: 0.0,
            tint: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            gamma: 1.0,
            saturation: 1.0,
            vibrance: 0.0,
            tone_curve: None,
        }
    }

    #[test]
    fn identity_params_do_nothing() {
        let params = identity();
        assert!(params.is_identity());
        assert_eq!(params.apply_reference([100, 150, 200]), [100, 150, 200]);
    }

    #[test]
    fn exposure_doubles_linear_light() {
        let params = ColorParams {
            exposure: 1.0,
            ..identity()
        };
        let out = params.tonal_chain(0, 0.4);
        let linear_in = transfer::srgb_to_linear(0.4);
        let linear_out = transfer::srgb_to_linear(out);
        assert_relative_eq!(linear_out, 2.0 * linear_in, epsilon = 1e-5);
    }

    #[test]
    fn warm_temperature_shifts_red_over_blue() {
        let params = ColorParams {
            temperature: 50.0,
            ..identity()
        };
        let [r, g, b] = params.apply_reference([128, 128, 128]);
        assert!(r > g, "warm should boost red: {r} vs {g}");
        assert!(b < g, "warm should cut blue: {b} vs {g}");
    }

    #[test]
    fn positive_tint_cuts_green() {
        let params = ColorParams {
            tint: 50.0,
            ..identity()
        };
        let [r, g, b] = params.apply_reference([128, 128, 128]);
        assert!(g < r && g < b, "magenta tint should cut green: {r},{g},{b}");
    }

    #[test]
    fn negative_highlights_darken_bright_tones_only() {
        let params = ColorParams {
            highlights: -100.0,
            ..identity()
        };
        // Bright tone pulled down.
        assert!(params.tonal_chain(0, 0.9) < 0.9);
        // Dark tone untouched (weight is zero below mid-gray).
        assert_relative_eq!(params.tonal_chain(0, 0.2), 0.2, epsilon = 1e-4);
    }

    #[test]
    fn positive_shadows_lift_dark_tones_only() {
        let params = ColorParams {
            shadows: 100.0,
            ..identity()
        };
        assert!(params.tonal_chain(0, 0.1) > 0.1);
        assert_relative_eq!(params.tonal_chain(0, 0.9), 0.9, epsilon = 1e-4);
    }

    #[test]
    fn gamma_brightens_midtones_fixes_endpoints() {
        let params = ColorParams {
            gamma: 2.0,
            ..identity()
        };
        assert_relative_eq!(params.tonal_chain(0, 0.0), 0.0, epsilon = 1e-5);
        assert_relative_eq!(params.tonal_chain(0, 1.0), 1.0, epsilon = 1e-5);
        assert!(params.tonal_chain(0, 0.5) > 0.5);
    }

    #[test]
    fn tone_curve_applies_in_display_space() {
        let curve = ToneCurve {
            points: vec![(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)],
        };
        let params = ColorParams {
            tone_curve: Some(&curve),
            ..identity()
        };
        assert_relative_eq!(params.tonal_chain(0, 0.5), 0.7, epsilon = 1e-5);
    }

    #[test]
    fn saturation_zero_is_grayscale() {
        let params = ColorParams {
            saturation: 0.0,
            ..identity()
        };
        let [r, g, b] = params.apply_reference([200, 30, 90]);
        assert_eq!(r, g);
        assert_eq!(g, b);
    }

    #[test]
    fn vibrance_boosts_muted_more_than_saturated() {
        let params = ColorParams {
            vibrance: 100.0,
            ..identity()
        };
        // Muted color: small channel spread grows noticeably.
        let muted_in = [140u8, 128, 120];
        let muted_out = params.apply_reference(muted_in);
        let spread_in = muted_in[0] as i16 - muted_in[2] as i16;
        let spread_out = muted_out[0] as i16 - muted_out[2] as i16;
        assert!(spread_out > spread_in);

        // Fully saturated color barely changes.
        let sat_out = params.apply_reference([255, 0, 0]);
        assert_eq!(sat_out, [255, 0, 0]);
    }

    #[test]
    fn contrast_pivots_at_mid_gray() {
        let params = ColorParams {
            contrast: 2.0,
            ..identity()
        };
        assert_relative_eq!(params.tonal_chain(0, 0.5), 0.5, epsilon = 1e-6);
        assert!(params.tonal_chain(0, 0.7) > 0.7);
        assert!(params.tonal_chain(0, 0.3) < 0.3);
    }
}
