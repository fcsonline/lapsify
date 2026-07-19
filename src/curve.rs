use crate::error::{LapsifyError, Result};

/// Parse a comma-separated list of floats, e.g. "-1.0,0.5,1.0".
pub fn parse_value_array(input: &str) -> Result<Vec<f32>> {
    input
        .split(',')
        .map(|s| s.trim().parse::<f32>())
        .collect::<std::result::Result<Vec<f32>, _>>()
        .map_err(|e| LapsifyError::message(format!("Failed to parse value array: {e}")))
}

pub fn validate_value_array(values: &[f32], name: &'static str, min: f32, max: f32) -> Result<()> {
    for (i, &value) in values.iter().enumerate() {
        if value < min || value > max {
            return Err(LapsifyError::InvalidParam {
                field: name,
                reason: format!(
                    "value at index {i} ({value}) is outside valid range [{min}, {max}]"
                ),
            });
        }
    }
    Ok(())
}

pub fn interpolate_value(values: &[f32], frame_index: usize, total_frames: usize) -> f32 {
    if values.len() == 1 || total_frames <= 1 {
        values[0]
    } else if values.len() == 2 {
        let t = frame_index as f32 / (total_frames - 1) as f32;
        values[0] + (values[1] - values[0]) * t
    } else {
        let t = frame_index as f32 / (total_frames - 1) as f32;
        bezier_interpolate(values, t)
    }
}

/// Bezier curve interpolation using Bernstein polynomials.
fn bezier_interpolate(control_points: &[f32], t: f32) -> f32 {
    let n = control_points.len() - 1;
    if n == 0 {
        return control_points[0];
    }

    let mut result = 0.0;
    for (i, &point) in control_points.iter().enumerate() {
        let coefficient = binomial_coefficient(n, i) as f32;
        result += coefficient * point * (1.0 - t).powi((n - i) as i32) * t.powi(i as i32);
    }
    result
}

fn binomial_coefficient(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    if k == 0 || k == n {
        return 1;
    }

    let mut result = 1;
    for i in 0..k {
        result = result * (n - i) / (i + 1);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

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

    #[test]
    fn validate_value_array_bounds() {
        assert!(validate_value_array(&[0.0, 1.0], "test", -1.0, 1.0).is_ok());
        assert!(validate_value_array(&[2.0], "test", -1.0, 1.0).is_err());
    }

    #[test]
    fn interpolate_single_value_is_constant() {
        assert_relative_eq!(interpolate_value(&[0.7], 0, 10), 0.7);
        assert_relative_eq!(interpolate_value(&[0.7], 9, 10), 0.7);
    }

    #[test]
    fn interpolate_two_values_is_linear() {
        assert_relative_eq!(interpolate_value(&[0.0, 1.0], 0, 11), 0.0);
        assert_relative_eq!(interpolate_value(&[0.0, 1.0], 5, 11), 0.5);
        assert_relative_eq!(interpolate_value(&[0.0, 1.0], 10, 11), 1.0);
    }

    #[test]
    fn interpolate_single_frame_clip_returns_first_value() {
        // A 1-frame clip must not divide by zero.
        let v = interpolate_value(&[0.0, 1.0], 0, 1);
        assert!(v.is_finite());
        assert_relative_eq!(v, 0.0);
    }

    #[test]
    fn bezier_hits_endpoints() {
        let points = [0.0, 5.0, 1.0];
        assert_relative_eq!(interpolate_value(&points, 0, 100), 0.0);
        assert_relative_eq!(interpolate_value(&points, 99, 100), 1.0, epsilon = 1e-4);
    }

    #[test]
    fn binomial_coefficients() {
        assert_eq!(binomial_coefficient(4, 0), 1);
        assert_eq!(binomial_coefficient(4, 2), 6);
        assert_eq!(binomial_coefficient(4, 4), 1);
        assert_eq!(binomial_coefficient(2, 3), 0);
    }
}
