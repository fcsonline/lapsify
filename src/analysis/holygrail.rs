//! Day-to-night ("holy grail") exposure compensation from camera EXIF.
//!
//! When shutter/aperture/ISO change during a shoot, the captured brightness
//! jumps by the same number of stops. The compensation layer is the
//! cumulative inverse of those camera exposure steps, so the develop-side
//! exposure cancels each jump and the remaining brightness ramp stays smooth
//! and keyframeable.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::analysis::{now_unix, source_fingerprint};
use crate::error::{LapsifyError, Result};
use crate::exif::{camera_ev, read_frame_exif};
use crate::progress::{ProgressEvent, ProgressReporter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolyGrailLayer {
    /// Cumulative compensation in EV stops: raw[i] = EV_cam[i] − EV_cam[0].
    /// +1.0 means the camera captured one stop darker than frame 0, so the
    /// develop exposure compensates by +1 EV.
    pub raw: Vec<f32>,
    /// Linear baseline tilt over the normalized clip position (0..1).
    pub rotate: f32,
    /// Scale of the whole compensation.
    pub stretch: f32,
    /// Frames whose EXIF was missing (their step was carried forward).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames_missing_exif: Vec<u32>,
    pub computed_at_unix: u64,
    pub source_fingerprint: String,
}

impl HolyGrailLayer {
    /// The effective compensation for a frame, in EV stops.
    pub fn effective(&self, frame: usize) -> f32 {
        let n = self.raw.len();
        if n == 0 {
            return 0.0;
        }
        let i = frame.min(n - 1);
        let u = if n <= 1 {
            0.0
        } else {
            i as f32 / (n - 1) as f32
        };
        self.stretch * (self.raw[i] + self.rotate * u)
    }
}

pub struct HolyGrailOptions {
    /// Manual rotate override. None = auto-fit so the compensation ends at 0.
    pub rotate: Option<f32>,
    /// Manual stretch override. None = 1.0.
    pub stretch: Option<f32>,
}

/// The pure staircase inversion: per-frame camera EVs (None = missing EXIF)
/// to cumulative compensation values with carry-forward for gaps.
pub fn layer_from_evs(evs: &[Option<f32>]) -> Result<(Vec<f32>, Vec<u32>)> {
    let usable = evs.iter().flatten().count();
    if usable < 2 {
        return Err(LapsifyError::message(
            "No usable EXIF exposure data; holy grail analysis needs shutter, aperture and ISO in at least 2 frames",
        ));
    }

    let mut raw = Vec::with_capacity(evs.len());
    let mut missing = Vec::new();
    let mut base: Option<f32> = None;
    let mut previous = 0.0f32;

    for (i, ev) in evs.iter().enumerate() {
        let value = match (ev, base) {
            (Some(ev), Some(base)) => ev - base,
            (Some(ev), None) => {
                base = Some(*ev);
                0.0
            }
            (None, _) => {
                missing.push(i as u32);
                previous
            }
        };
        raw.push(value);
        previous = value;
    }

    Ok((raw, missing))
}

/// Read EXIF across the sequence and build the compensation layer plus
/// capture timestamps (missing timestamps are interpolated between
/// neighbors).
pub fn compute_holy_grail(
    image_files: &[PathBuf],
    opts: &HolyGrailOptions,
    reporter: &ProgressReporter,
) -> Result<(HolyGrailLayer, Option<Vec<i64>>)> {
    let total = image_files.len();
    let done = AtomicUsize::new(0);

    let exifs: Vec<crate::exif::FrameExif> = image_files
        .par_iter()
        .map(|path| {
            let exif = read_frame_exif(path);
            let current = done.fetch_add(1, Ordering::Relaxed) + 1;
            reporter.report(ProgressEvent::Frame {
                index: current - 1,
                done: current,
                total,
            });
            exif
        })
        .collect();

    let evs: Vec<Option<f32>> = exifs.iter().map(camera_ev).collect();
    let (raw, missing) = layer_from_evs(&evs)?;

    let rotate = opts.rotate.unwrap_or_else(|| {
        // Auto-fit: tilt the baseline so the compensation ends at zero and
        // the natural day-night ramp stays in the keyframes' hands.
        -raw.last().copied().unwrap_or(0.0)
    });
    let stretch = opts.stretch.unwrap_or(1.0);

    let layer = HolyGrailLayer {
        raw,
        rotate,
        stretch,
        frames_missing_exif: missing,
        computed_at_unix: now_unix(),
        source_fingerprint: source_fingerprint(image_files)?,
    };

    let times = interpolate_times(exifs.iter().map(|e| e.datetime_ms).collect());

    Ok((layer, times))
}

/// Fill missing capture timestamps by linear interpolation between known
/// neighbors (edges extend the nearest known spacing). None if no frame has
/// a timestamp.
fn interpolate_times(times: Vec<Option<i64>>) -> Option<Vec<i64>> {
    let known: Vec<(usize, i64)> = times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| t.map(|t| (i, t)))
        .collect();
    if known.is_empty() {
        return None;
    }

    let n = times.len();
    let mut out = vec![0i64; n];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = match times[i] {
            Some(t) => t,
            None => {
                let before = known.iter().rev().find(|(k, _)| *k < i);
                let after = known.iter().find(|(k, _)| *k > i);
                match (before, after) {
                    (Some(&(i0, t0)), Some(&(i1, t1))) => {
                        t0 + (t1 - t0) * (i - i0) as i64 / (i1 - i0) as i64
                    }
                    (Some(&(_, t0)), None) => t0,
                    (None, Some(&(_, t1))) => t1,
                    (None, None) => unreachable!("known is non-empty"),
                }
            }
        };
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn staircase_inversion_cancels_jumps() {
        // Camera EV drops one stop at frame 2 (lets in more light -> image
        // jumps brighter) and another at frame 4.
        let evs = vec![
            Some(10.0),
            Some(10.0),
            Some(9.0),
            Some(9.0),
            Some(8.0),
            Some(8.0),
        ];
        let (raw, missing) = layer_from_evs(&evs).unwrap();
        assert_eq!(raw, vec![0.0, 0.0, -1.0, -1.0, -2.0, -2.0]);
        assert!(missing.is_empty());
    }

    #[test]
    fn missing_exif_carries_forward() {
        let evs = vec![Some(10.0), None, Some(9.0), None];
        let (raw, missing) = layer_from_evs(&evs).unwrap();
        assert_eq!(raw, vec![0.0, 0.0, -1.0, -1.0]);
        assert_eq!(missing, vec![1, 3]);
    }

    #[test]
    fn too_little_exif_is_an_error() {
        assert!(layer_from_evs(&[None, None, Some(9.0)]).is_err());
        assert!(layer_from_evs(&[]).is_err());
    }

    #[test]
    fn auto_fit_rotate_lands_the_end_at_zero() {
        let layer = HolyGrailLayer {
            raw: vec![0.0, -1.0, -2.0, -3.0],
            rotate: 3.0, // = -raw.last()
            stretch: 1.0,
            frames_missing_exif: vec![],
            computed_at_unix: 0,
            source_fingerprint: String::new(),
        };
        assert_relative_eq!(layer.effective(0), 0.0);
        assert_relative_eq!(layer.effective(3), 0.0);
        // Interior frames keep the jump-cancelling shape minus the tilt.
        assert_relative_eq!(layer.effective(1), -1.0 + 1.0);
        assert_relative_eq!(layer.effective(2), -2.0 + 2.0);
    }

    #[test]
    fn effective_clamps_beyond_range_and_handles_empty() {
        let layer = HolyGrailLayer {
            raw: vec![0.0, -1.0],
            rotate: 0.0,
            stretch: 1.0,
            frames_missing_exif: vec![],
            computed_at_unix: 0,
            source_fingerprint: String::new(),
        };
        assert_relative_eq!(layer.effective(99), -1.0);

        let empty = HolyGrailLayer {
            raw: vec![],
            rotate: 0.0,
            stretch: 1.0,
            frames_missing_exif: vec![],
            computed_at_unix: 0,
            source_fingerprint: String::new(),
        };
        assert_relative_eq!(empty.effective(0), 0.0);
    }

    #[test]
    fn time_interpolation_fills_gaps() {
        let times = vec![Some(1000), None, None, Some(4000)];
        assert_eq!(
            interpolate_times(times).unwrap(),
            vec![1000, 2000, 3000, 4000]
        );
        assert_eq!(interpolate_times(vec![None, None]), None);
        assert_eq!(
            interpolate_times(vec![None, Some(5000), None]).unwrap(),
            vec![5000, 5000, 5000]
        );
    }
}
