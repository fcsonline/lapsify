//! Visual deflicker: measure the developed luminance of every frame, build a
//! smoothed target curve, and correct each frame toward the target with a
//! per-frame exposure offset. The pipeline is nonlinear, so correction is
//! iterative: re-develop, re-measure, refine, until every frame lands within
//! a threshold of the target.
//!
//! Idempotency: offsets are absolute corrections against a stored target,
//! and the target is computed from the deflicker-free luminance
//! `L0 = L_measured / 2^offset`. L0 is invariant under the correction
//! itself, so re-running the command never re-smooths its own output.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::analysis::luminance::{measure_luminance, LuminanceOptions};
use crate::analysis::{now_unix, source_fingerprint, Analysis};
use crate::crop::CropRect;
use crate::error::Result;
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::project::Project;

/// Guard for log2 on near-black frames.
const LUMA_FLOOR: f32 = 1e-6;
/// Per-pass correction step limit, in EV, for stability.
const MAX_STEP_EV: f32 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeflickerLayer {
    /// The target luminance curve (smoothed deflicker-free luminance).
    pub target: Vec<f32>,
    /// Absolute per-frame exposure corrections in EV stops.
    pub offsets: Vec<f32>,
    pub smoothing_frames: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<CropRect>,
    pub threshold_ev: f32,
    pub passes_run: u32,
    pub converged: bool,
    pub computed_at_unix: u64,
    pub source_fingerprint: String,
}

impl DeflickerLayer {
    pub fn offset(&self, frame: usize) -> f32 {
        if self.offsets.is_empty() {
            0.0
        } else {
            self.offsets[frame.min(self.offsets.len() - 1)]
        }
    }
}

pub struct DeflickerOptions {
    /// Low-pass window in frames for the target curve.
    pub smoothing_frames: u32,
    /// Normalized source-image region to measure. None = full frame.
    pub region: Option<CropRect>,
    /// Maximum correction passes.
    pub max_passes: u32,
    /// Per-frame convergence threshold in EV.
    pub threshold_ev: f32,
    /// Thumbnail size for measurement.
    pub measure_dim: u32,
    /// Keep the stored target curve instead of recomputing it.
    pub refine: bool,
}

impl Default for DeflickerOptions {
    fn default() -> Self {
        Self {
            smoothing_frames: 30,
            region: None,
            max_passes: 3,
            threshold_ev: 0.03,
            measure_dim: 256,
            refine: false,
        }
    }
}

/// Run the deflicker loop against a project, returning the finished layer.
/// The passed project is used as the grading definition; its existing
/// deflicker offsets (if any) are the starting point.
pub fn run_deflicker(
    project: &Project,
    image_files: &[PathBuf],
    opts: &DeflickerOptions,
    reporter: &ProgressReporter,
) -> Result<DeflickerLayer> {
    let n = image_files.len();
    let fingerprint = source_fingerprint(image_files)?;

    // Work on a copy so the measurement pass sees the evolving offsets.
    let mut working = project.clone();
    let analysis = working.analysis.get_or_insert_with(Analysis::default);
    let mut layer = match analysis.deflicker.take() {
        Some(existing) if existing.offsets.len() == n => existing,
        _ => DeflickerLayer {
            target: Vec::new(),
            offsets: vec![0.0; n],
            smoothing_frames: opts.smoothing_frames,
            region: opts.region,
            threshold_ev: opts.threshold_ev,
            passes_run: 0,
            converged: false,
            computed_at_unix: 0,
            source_fingerprint: String::new(),
        },
    };
    if !opts.refine || layer.target.len() != n {
        layer.target.clear();
    }

    let luma_opts = LuminanceOptions {
        region: opts.region,
        measure_dim: opts.measure_dim,
        developed: true,
    };

    let mut passes_run = 0;
    let mut converged = false;

    for pass in 0..opts.max_passes {
        // Measure developed luminance with the current offsets applied.
        working.analysis.as_mut().unwrap().deflicker = Some(layer.clone());
        let measured = measure_luminance(&working, image_files, &luma_opts, reporter)?;

        // Divide the correction back out: L0 is what the graded frame would
        // show with no deflicker, regardless of the current offsets.
        let l0: Vec<f32> = measured
            .values
            .iter()
            .zip(&layer.offsets)
            .map(|(l, off)| (l / 2.0_f32.powf(*off)).max(LUMA_FLOOR))
            .collect();

        if layer.target.is_empty() {
            layer.target = gaussian_smooth(&l0, opts.smoothing_frames as f32 / 4.0)
                .into_iter()
                .map(|v| v.max(LUMA_FLOOR))
                .collect();
        }

        let mut max_delta = 0.0f32;
        let mut corrected = 0usize;
        for ((offset, target), l0_value) in layer.offsets.iter_mut().zip(&layer.target).zip(&l0) {
            let wanted = (target / l0_value).log2();
            let delta = (wanted - *offset).clamp(-MAX_STEP_EV, MAX_STEP_EV);
            if delta.abs() >= opts.threshold_ev {
                corrected += 1;
            }
            max_delta = max_delta.max(delta.abs());
            *offset += delta;
        }

        passes_run = pass + 1;
        reporter.report(ProgressEvent::DeflickerPass {
            pass: passes_run,
            frames_corrected: corrected,
            max_delta_ev: max_delta,
        });

        if max_delta < opts.threshold_ev {
            converged = true;
            break;
        }
    }

    layer.smoothing_frames = opts.smoothing_frames;
    layer.region = opts.region;
    layer.threshold_ev = opts.threshold_ev;
    layer.passes_run = passes_run;
    layer.converged = converged;
    layer.computed_at_unix = now_unix();
    layer.source_fingerprint = fingerprint;

    Ok(layer)
}

/// Gaussian smoothing with edge clamping.
pub fn gaussian_smooth(values: &[f32], sigma: f32) -> Vec<f32> {
    if values.is_empty() || sigma <= 0.0 {
        return values.to_vec();
    }

    let radius = (3.0 * sigma).ceil() as i64;
    let kernel: Vec<f32> = (-radius..=radius)
        .map(|d| (-((d * d) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();

    let n = values.len() as i64;
    (0..n)
        .map(|i| {
            let mut sum = 0.0f32;
            let mut weight = 0.0f32;
            for (k, w) in kernel.iter().enumerate() {
                let j = (i + k as i64 - radius).clamp(0, n - 1) as usize;
                sum += values[j] * w;
                weight += w;
            }
            sum / weight
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn smoothing_removes_jitter_keeps_trend() {
        // A ramp with alternating jitter.
        let values: Vec<f32> = (0..100)
            .map(|i| i as f32 / 100.0 + if i % 2 == 0 { 0.05 } else { -0.05 })
            .collect();
        let smoothed = gaussian_smooth(&values, 5.0);

        // Jitter gone: neighboring samples are close.
        for pair in smoothed.windows(2) {
            assert!((pair[1] - pair[0]).abs() < 0.03);
        }
        // Trend kept: still spans most of the ramp.
        assert!(smoothed[95] - smoothed[5] > 0.7);
    }

    #[test]
    fn smoothing_handles_edges_and_empty() {
        assert!(gaussian_smooth(&[], 3.0).is_empty());
        let constant = gaussian_smooth(&[2.0; 10], 3.0);
        for v in constant {
            assert_relative_eq!(v, 2.0, epsilon = 1e-5);
        }
        assert_eq!(gaussian_smooth(&[1.0, 2.0], 0.0), vec![1.0, 2.0]);
    }

    #[test]
    fn layer_offset_clamps_and_handles_empty() {
        let layer = DeflickerLayer {
            target: vec![],
            offsets: vec![0.1, -0.2],
            smoothing_frames: 30,
            region: None,
            threshold_ev: 0.01,
            passes_run: 1,
            converged: true,
            computed_at_unix: 0,
            source_fingerprint: String::new(),
        };
        assert_relative_eq!(layer.offset(0), 0.1);
        assert_relative_eq!(layer.offset(99), -0.2);

        let empty = DeflickerLayer {
            offsets: vec![],
            ..layer
        };
        assert_relative_eq!(empty.offset(0), 0.0);
    }
}
