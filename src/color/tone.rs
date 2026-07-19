use serde::{Deserialize, Serialize};

use crate::error::{LapsifyError, Result};

/// A parametric tone curve: monotone cubic interpolation through control
/// points in the display-referred 0..=1 domain. Static across the clip
/// (tone curve shapes are copied, not interpolated, between frames).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ToneCurve {
    /// Control points (input, output), both 0..=1, sorted by input.
    pub points: Vec<(f32, f32)>,
}

impl ToneCurve {
    pub fn validate(&self) -> Result<()> {
        if self.points.len() < 2 {
            return Err(LapsifyError::InvalidParam {
                field: "tone_curve",
                reason: "needs at least 2 control points".to_string(),
            });
        }
        for &(x, y) in &self.points {
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return Err(LapsifyError::InvalidParam {
                    field: "tone_curve",
                    reason: format!("control point ({x}, {y}) is outside 0..=1"),
                });
            }
        }
        for pair in self.points.windows(2) {
            if pair[1].0 <= pair[0].0 {
                return Err(LapsifyError::InvalidParam {
                    field: "tone_curve",
                    reason: "control point inputs must be strictly increasing".to_string(),
                });
            }
        }
        Ok(())
    }

    /// Evaluate the curve. Inputs outside the control range clamp to the
    /// first/last output value.
    pub fn sample(&self, x: f32) -> f32 {
        let points = &self.points;
        let first = points[0];
        let last = points[points.len() - 1];
        if x <= first.0 {
            return first.1;
        }
        if x >= last.0 {
            return last.1;
        }

        let i = points.partition_point(|p| p.0 <= x) - 1;
        let (x0, y0) = points[i];
        let (x1, y1) = points[i + 1];

        let tangents = monotone_tangents(points);
        let h = x1 - x0;
        let t = (x - x0) / h;
        let t2 = t * t;
        let t3 = t2 * t;
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        (h00 * y0 + h10 * h * tangents[i] + h01 * y1 + h11 * h * tangents[i + 1]).clamp(0.0, 1.0)
    }
}

/// Fritsch-Carlson tangents over (x, y) control points.
fn monotone_tangents(points: &[(f32, f32)]) -> Vec<f32> {
    let n = points.len();
    let mut secants = Vec::with_capacity(n - 1);
    for pair in points.windows(2) {
        secants.push((pair[1].1 - pair[0].1) / (pair[1].0 - pair[0].0));
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

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn identity_curve_is_identity() {
        let curve = ToneCurve {
            points: vec![(0.0, 0.0), (1.0, 1.0)],
        };
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert_relative_eq!(curve.sample(x), x, epsilon = 1e-6);
        }
    }

    #[test]
    fn s_curve_passes_through_points_and_stays_monotone() {
        let curve = ToneCurve {
            points: vec![(0.0, 0.0), (0.25, 0.15), (0.75, 0.85), (1.0, 1.0)],
        };
        assert!(curve.validate().is_ok());
        assert_relative_eq!(curve.sample(0.25), 0.15, epsilon = 1e-6);
        assert_relative_eq!(curve.sample(0.75), 0.85, epsilon = 1e-6);

        let mut prev = 0.0;
        for i in 0..=100 {
            let v = curve.sample(i as f32 / 100.0);
            assert!(v >= prev - 1e-5, "not monotone at {i}");
            prev = v;
        }
    }

    #[test]
    fn validate_rejects_bad_curves() {
        assert!(ToneCurve {
            points: vec![(0.0, 0.0)]
        }
        .validate()
        .is_err());
        assert!(ToneCurve {
            points: vec![(0.5, 0.0), (0.2, 1.0)]
        }
        .validate()
        .is_err());
        assert!(ToneCurve {
            points: vec![(0.0, 0.0), (1.5, 1.0)]
        }
        .validate()
        .is_err());
    }
}
