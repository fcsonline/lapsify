//! sRGB transfer functions (IEC 61966-2-1).

/// Decode a gamma-encoded sRGB component (0..=1) to linear light.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Encode linear light (0..=1) to a gamma-encoded sRGB component.
pub fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.0031308 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// Decode table for 8-bit sRGB input.
pub fn srgb_decode_table() -> [f32; 256] {
    let mut table = [0.0f32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        *entry = srgb_to_linear(i as f32 / 255.0);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn round_trip_is_identity() {
        for i in 0..=1000 {
            let c = i as f32 / 1000.0;
            assert_relative_eq!(linear_to_srgb(srgb_to_linear(c)), c, epsilon = 1e-5);
        }
    }

    #[test]
    fn known_anchor_points() {
        assert_relative_eq!(srgb_to_linear(0.0), 0.0);
        assert_relative_eq!(srgb_to_linear(1.0), 1.0, epsilon = 1e-6);
        // 18% gray card is about 46.6% in sRGB.
        assert_relative_eq!(linear_to_srgb(0.18), 0.4613561, epsilon = 1e-5);
    }

    #[test]
    fn decode_table_matches_function() {
        let table = srgb_decode_table();
        assert_relative_eq!(table[0], 0.0);
        assert_relative_eq!(table[255], 1.0, epsilon = 1e-6);
        assert_relative_eq!(table[128], srgb_to_linear(128.0 / 255.0));
    }
}
