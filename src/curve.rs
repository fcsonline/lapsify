use serde::{Deserialize, Serialize};

use crate::error::{LapsifyError, Result};

/// Easing applied to the segment that leaves a keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    /// Monotone cubic interpolation through all keyframes (no overshoot).
    #[default]
    Smooth,
    Linear,
    /// Keep the previous keyframe's value until the next keyframe.
    Hold,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub frame: u32,
    pub value: f32,
    #[serde(default, skip_serializing_if = "is_default_easing")]
    pub easing: Easing,
}

fn is_default_easing(easing: &Easing) -> bool {
    *easing == Easing::Smooth
}

impl Keyframe {
    pub fn new(frame: u32, value: f32) -> Self {
        Self {
            frame,
            value,
            easing: Easing::Smooth,
        }
    }
}

/// A parameter value over time: either constant for the whole clip or a set
/// of keyframes anchored to specific frames.
///
/// Serializes as a bare number ("exposure": 0.5) or a keyframe list
/// ("exposure": [{"frame": 0, "value": 0.0}, {"frame": 120, "value": 1.5}]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Curve {
    Constant(f32),
    Keyframed(Vec<Keyframe>),
}

impl Curve {
    /// Sample the curve at a frame. Frames outside the keyframe range clamp
    /// to the first/last keyframe value.
    pub fn sample(&self, frame: u32) -> f32 {
        self.sample_mapped(frame, |f| f as f32)
    }

    /// Sample the curve with an arbitrary frame-to-position mapping. With a
    /// capture-time mapping, interpolation happens in time space, which is
    /// what irregular shooting intervals need.
    pub fn sample_mapped(&self, frame: u32, x_of: impl Fn(u32) -> f32) -> f32 {
        match self {
            Curve::Constant(v) => *v,
            Curve::Keyframed(keyframes) => sample_keyframes(keyframes, frame, x_of),
        }
    }

    /// All control values of the curve. Because interpolation is monotone
    /// between keyframes, every sampled value lies within the min/max of
    /// these values, which makes them sufficient for range validation.
    pub fn values(&self) -> Vec<f32> {
        match self {
            Curve::Constant(v) => vec![*v],
            Curve::Keyframed(keyframes) => keyframes.iter().map(|k| k.value).collect(),
        }
    }

    pub fn validate(&self, name: &'static str) -> Result<()> {
        if let Curve::Keyframed(keyframes) = self {
            if keyframes.is_empty() {
                return Err(LapsifyError::InvalidParam {
                    field: name,
                    reason: "keyframed curve must have at least one keyframe".to_string(),
                });
            }
            for pair in keyframes.windows(2) {
                if pair[1].frame <= pair[0].frame {
                    return Err(LapsifyError::InvalidParam {
                        field: name,
                        reason: format!(
                            "keyframes must be sorted by frame with no duplicates (frame {} follows frame {})",
                            pair[1].frame, pair[0].frame
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validate that every control value lies inside [min, max].
    pub fn validate_range(&self, name: &'static str, min: f32, max: f32) -> Result<()> {
        for value in self.values() {
            if value < min || value > max {
                return Err(LapsifyError::InvalidParam {
                    field: name,
                    reason: format!("value {value} is outside valid range [{min}, {max}]"),
                });
            }
        }
        Ok(())
    }
}

fn sample_keyframes(keyframes: &[Keyframe], frame: u32, x_of: impl Fn(u32) -> f32) -> f32 {
    match keyframes {
        [] => 0.0,
        [only] => only.value,
        _ => {
            let first = &keyframes[0];
            let last = &keyframes[keyframes.len() - 1];
            if frame <= first.frame {
                return first.value;
            }
            if frame >= last.frame {
                return last.value;
            }

            // Index of the keyframe starting the segment containing `frame`.
            let i = keyframes.partition_point(|k| k.frame <= frame) - 1;
            let k0 = &keyframes[i];
            let k1 = &keyframes[i + 1];
            let x0 = x_of(k0.frame);
            let x1 = x_of(k1.frame);
            let span = x1 - x0;
            let t = if span > 0.0 {
                ((x_of(frame) - x0) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };

            match k0.easing {
                Easing::Hold => k0.value,
                Easing::Linear => lerp(k0.value, k1.value, t),
                Easing::EaseIn => lerp(k0.value, k1.value, t * t * t),
                Easing::EaseOut => {
                    let u = 1.0 - t;
                    lerp(k0.value, k1.value, 1.0 - u * u * u)
                }
                Easing::EaseInOut => lerp(k0.value, k1.value, t * t * (3.0 - 2.0 * t)),
                Easing::Smooth => {
                    let tangents = monotone_tangents(keyframes, &x_of);
                    hermite(k0, k1, tangents[i], tangents[i + 1], t, span)
                }
            }
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Fritsch-Carlson tangents for monotone cubic interpolation: the resulting
/// spline passes through every keyframe and never overshoots the interval
/// between neighboring keyframe values. Spacing comes from the position
/// mapping, so irregular capture intervals produce correct tangents.
fn monotone_tangents(keyframes: &[Keyframe], x_of: &impl Fn(u32) -> f32) -> Vec<f32> {
    let n = keyframes.len();
    let mut secants = Vec::with_capacity(n - 1);
    for pair in keyframes.windows(2) {
        let dx = (x_of(pair[1].frame) - x_of(pair[0].frame)).max(f32::EPSILON);
        secants.push((pair[1].value - pair[0].value) / dx);
    }

    let mut tangents = vec![0.0f32; n];
    tangents[0] = secants[0];
    tangents[n - 1] = secants[n - 2];
    for i in 1..n - 1 {
        if secants[i - 1] * secants[i] <= 0.0 {
            tangents[i] = 0.0;
        } else {
            tangents[i] = (secants[i - 1] + secants[i]) / 2.0;
        }
    }

    for i in 0..n - 1 {
        if secants[i] == 0.0 {
            tangents[i] = 0.0;
            tangents[i + 1] = 0.0;
        } else {
            let alpha = tangents[i] / secants[i];
            let beta = tangents[i + 1] / secants[i];
            let s = alpha * alpha + beta * beta;
            if s > 9.0 {
                let tau = 3.0 / s.sqrt();
                tangents[i] = tau * alpha * secants[i];
                tangents[i + 1] = tau * beta * secants[i];
            }
        }
    }

    tangents
}

fn hermite(k0: &Keyframe, k1: &Keyframe, m0: f32, m1: f32, t: f32, h: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * k0.value + h10 * h * m0 + h01 * k1.value + h11 * h * m1
}

/// Convert a legacy comma-array value ("-1.0,0.5,1.0") into a curve: values
/// are spread evenly over the clip as smooth keyframes.
pub fn curve_from_legacy_array(values: &[f32], total_frames: usize) -> Curve {
    if values.len() == 1 || total_frames <= 1 {
        return Curve::Constant(values[0]);
    }
    let last_frame = (total_frames - 1) as f32;
    let keyframes = values
        .iter()
        .enumerate()
        .map(|(i, &value)| {
            let frame = (i as f32 * last_frame / (values.len() - 1) as f32).round() as u32;
            Keyframe::new(frame, value)
        })
        .collect();
    Curve::Keyframed(keyframes)
}

/// Parse a comma-separated list of floats, e.g. "-1.0,0.5,1.0".
pub fn parse_value_array(input: &str) -> Result<Vec<f32>> {
    input
        .split(',')
        .map(|s| s.trim().parse::<f32>())
        .collect::<std::result::Result<Vec<f32>, _>>()
        .map_err(|e| LapsifyError::message(format!("Failed to parse value array: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn keyframed(points: &[(u32, f32)]) -> Curve {
        Curve::Keyframed(points.iter().map(|&(f, v)| Keyframe::new(f, v)).collect())
    }

    #[test]
    fn constant_samples_everywhere() {
        let curve = Curve::Constant(0.7);
        assert_relative_eq!(curve.sample(0), 0.7);
        assert_relative_eq!(curve.sample(1000), 0.7);
    }

    #[test]
    fn curve_passes_through_every_keyframe() {
        let points = [(0, 0.0), (10, 2.0), (20, -1.0), (35, 0.5)];
        let curve = keyframed(&points);
        for &(frame, value) in &points {
            assert_relative_eq!(curve.sample(frame), value, epsilon = 1e-6);
        }
    }

    #[test]
    fn clamps_outside_keyframe_range() {
        let curve = keyframed(&[(10, 1.0), (20, 2.0)]);
        assert_relative_eq!(curve.sample(0), 1.0);
        assert_relative_eq!(curve.sample(100), 2.0);
    }

    #[test]
    fn smooth_interpolation_never_overshoots() {
        // Monotone data must stay monotone; all samples within neighbor bounds.
        let curve = keyframed(&[(0, 0.0), (10, 0.1), (20, 5.0), (30, 5.1)]);
        let mut prev = curve.sample(0);
        for frame in 1..=30 {
            let v = curve.sample(frame);
            assert!(
                v >= prev - 1e-4,
                "not monotone at frame {frame}: {v} < {prev}"
            );
            assert!((-0.001..=5.101).contains(&v), "overshoot at {frame}: {v}");
            prev = v;
        }
    }

    #[test]
    fn linear_easing_is_linear() {
        let curve = Curve::Keyframed(vec![
            Keyframe {
                frame: 0,
                value: 0.0,
                easing: Easing::Linear,
            },
            Keyframe::new(10, 1.0),
        ]);
        assert_relative_eq!(curve.sample(5), 0.5);
    }

    #[test]
    fn hold_easing_steps() {
        let curve = Curve::Keyframed(vec![
            Keyframe {
                frame: 0,
                value: 1.0,
                easing: Easing::Hold,
            },
            Keyframe::new(10, 2.0),
        ]);
        assert_relative_eq!(curve.sample(9), 1.0);
        assert_relative_eq!(curve.sample(10), 2.0);
    }

    #[test]
    fn ease_in_out_hits_midpoint() {
        let curve = Curve::Keyframed(vec![
            Keyframe {
                frame: 0,
                value: 0.0,
                easing: Easing::EaseInOut,
            },
            Keyframe::new(10, 1.0),
        ]);
        assert_relative_eq!(curve.sample(0), 0.0);
        assert_relative_eq!(curve.sample(5), 0.5);
        assert_relative_eq!(curve.sample(10), 1.0);
    }

    #[test]
    fn single_keyframe_is_constant() {
        let curve = keyframed(&[(5, 3.0)]);
        assert_relative_eq!(curve.sample(0), 3.0);
        assert_relative_eq!(curve.sample(99), 3.0);
    }

    #[test]
    fn validate_rejects_unsorted_and_empty() {
        assert!(keyframed(&[(10, 0.0), (5, 1.0)]).validate("test").is_err());
        assert!(keyframed(&[(5, 0.0), (5, 1.0)]).validate("test").is_err());
        assert!(Curve::Keyframed(vec![]).validate("test").is_err());
        assert!(keyframed(&[(0, 0.0), (5, 1.0)]).validate("test").is_ok());
        assert!(Curve::Constant(1.0).validate("test").is_ok());
    }

    #[test]
    fn validate_range_checks_control_values() {
        assert!(Curve::Constant(5.0)
            .validate_range("test", -3.0, 3.0)
            .is_err());
        assert!(keyframed(&[(0, -1.0), (10, 1.0)])
            .validate_range("test", -3.0, 3.0)
            .is_ok());
    }

    #[test]
    fn legacy_array_conversion_spreads_keyframes() {
        let curve = curve_from_legacy_array(&[0.0, 1.0, 0.5], 101);
        match &curve {
            Curve::Keyframed(kfs) => {
                assert_eq!(kfs.len(), 3);
                assert_eq!(kfs[0].frame, 0);
                assert_eq!(kfs[1].frame, 50);
                assert_eq!(kfs[2].frame, 100);
            }
            _ => panic!("expected keyframed curve"),
        }
        // The curve passes through every legacy value.
        assert_relative_eq!(curve.sample(50), 1.0);

        assert_eq!(curve_from_legacy_array(&[0.5], 100), Curve::Constant(0.5));
        assert_eq!(
            curve_from_legacy_array(&[0.5, 1.0], 1),
            Curve::Constant(0.5)
        );
    }

    #[test]
    fn serde_roundtrip_constant_and_keyframed() {
        let constant: Curve = serde_json::from_str("0.5").unwrap();
        assert_eq!(constant, Curve::Constant(0.5));

        let json = r#"[{"frame":0,"value":0.0},{"frame":10,"value":1.0,"easing":"linear"}]"#;
        let curve: Curve = serde_json::from_str(json).unwrap();
        match &curve {
            Curve::Keyframed(kfs) => {
                assert_eq!(kfs[0].easing, Easing::Smooth);
                assert_eq!(kfs[1].easing, Easing::Linear);
            }
            _ => panic!("expected keyframed curve"),
        }

        let back = serde_json::to_string(&curve).unwrap();
        let reparsed: Curve = serde_json::from_str(&back).unwrap();
        assert_eq!(curve, reparsed);
    }

    #[test]
    fn parse_value_array_single_and_multi() {
        assert_eq!(parse_value_array("1.5").unwrap(), vec![1.5]);
        assert_eq!(
            parse_value_array("-1.0, 0.5 ,1.0").unwrap(),
            vec![-1.0, 0.5, 1.0]
        );
        assert!(parse_value_array("1.0,abc").is_err());
        assert!(parse_value_array("").is_err());
    }
}
