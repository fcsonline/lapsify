//! Keyframe placement suggestions from the luminance progression: denser
//! where brightness moves fast, sparse where nothing happens.

use crate::analysis::deflicker::gaussian_smooth;
use crate::analysis::holygrail::HolyGrailLayer;
use crate::error::{LapsifyError, Result};

const LUMA_FLOOR: f32 = 1e-6;

pub struct SuggestOptions {
    /// Exact number of keyframes. None = derive from density.
    pub count: Option<usize>,
    /// Keyframes per EV of total luminance travel (used when count is None).
    pub density: f32,
}

impl Default for SuggestOptions {
    fn default() -> Self {
        Self {
            count: None,
            density: 1.5,
        }
    }
}

/// Suggest keyframe positions from the source luminance progression.
///
/// The signal is source luminance in EV space, with the in-camera exposure
/// staircase cancelled (when a compensation layer exists) so camera-side
/// jumps don't masquerade as scene changes. Keyframes land at equal steps of
/// cumulative luminance travel: fast change gets dense keyframes.
pub fn suggest_keyframes(
    source_luma: &[f32],
    holy_grail: Option<&HolyGrailLayer>,
    opts: &SuggestOptions,
) -> Result<Vec<u32>> {
    let n = source_luma.len();
    if n < 2 {
        return Err(LapsifyError::message(
            "Keyframe suggestion needs at least 2 frames of luminance data",
        ));
    }

    // Luminance in EV space, camera staircase cancelled.
    let signal: Vec<f32> = source_luma
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut ev = l.max(LUMA_FLOOR).log2();
            if let Some(hg) = holy_grail {
                // raw[i] is the develop-side compensation for the camera's
                // step; adding it removes the step from the scene signal.
                ev += hg.raw.get(i).copied().unwrap_or(0.0);
            }
            ev
        })
        .collect();

    let smoothed = gaussian_smooth(&signal, (n as f32 * 0.02).max(1.0));

    // Cumulative absolute travel of the smoothed signal.
    let mut travel = Vec::with_capacity(n);
    let mut acc = 0.0f32;
    travel.push(0.0);
    for pair in smoothed.windows(2) {
        acc += (pair[1] - pair[0]).abs();
        travel.push(acc);
    }
    let total = acc;

    let count = match opts.count {
        Some(c) => c.max(2),
        None => ((total * opts.density).round() as usize).clamp(2, 64),
    };

    // Place keyframes at equal steps of cumulative travel. With zero travel
    // (static scene) this degrades to just the endpoints.
    let mut frames = vec![0u32];
    if total > 0.0 {
        let mut cursor = 0usize;
        for j in 1..count - 1 {
            let target = total * j as f32 / (count - 1) as f32;
            while cursor + 1 < n && travel[cursor] < target {
                cursor += 1;
            }
            let frame = cursor as u32;
            if *frames.last().unwrap() != frame && frame < (n - 1) as u32 {
                frames.push(frame);
            }
        }
    }
    frames.push((n - 1) as u32);

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_where_luminance_moves() {
        // Flat, then a fast 4 EV ramp between frames 40 and 60, then flat.
        let luma: Vec<f32> = (0..100)
            .map(|i| {
                let ev = ((i as f32 - 40.0) / 5.0).clamp(0.0, 4.0);
                0.02 * 2.0_f32.powf(ev)
            })
            .collect();

        let frames = suggest_keyframes(&luma, None, &SuggestOptions::default()).unwrap();

        assert_eq!(frames[0], 0);
        assert_eq!(*frames.last().unwrap(), 99);
        // The interior keyframes cluster inside the changing region.
        let interior: Vec<u32> = frames[1..frames.len() - 1].to_vec();
        assert!(!interior.is_empty());
        assert!(
            interior.iter().all(|&f| (35..=65).contains(&f)),
            "keyframes should cluster in the ramp: {interior:?}"
        );
    }

    #[test]
    fn static_scene_gets_only_endpoints() {
        let luma = vec![0.3f32; 50];
        let frames = suggest_keyframes(&luma, None, &SuggestOptions::default()).unwrap();
        assert_eq!(frames, vec![0, 49]);
    }

    #[test]
    fn count_is_respected() {
        let luma: Vec<f32> = (0..100)
            .map(|i| 0.1 * 2.0_f32.powf(i as f32 / 20.0))
            .collect();
        let frames = suggest_keyframes(
            &luma,
            None,
            &SuggestOptions {
                count: Some(5),
                density: 1.5,
            },
        )
        .unwrap();
        assert_eq!(frames.len(), 5);
    }

    #[test]
    fn camera_staircase_is_not_a_scene_change() {
        // Constant scene, but the camera opened up one stop at frame 25:
        // captured luminance jumps 2x. With the compensation layer the
        // signal is flat, so only endpoints are suggested.
        let mut luma = vec![0.2f32; 50];
        for v in luma.iter_mut().skip(25) {
            *v = 0.4;
        }
        let mut raw = vec![0.0f32; 25];
        raw.extend(vec![-1.0f32; 25]);
        let hg = HolyGrailLayer {
            raw,
            rotate: 0.0,
            stretch: 1.0,
            frames_missing_exif: vec![],
            computed_at_unix: 0,
            source_fingerprint: String::new(),
        };

        // With compensation the signal is flat: interior keyframes are
        // skipped even when explicitly asked for.
        let opts = SuggestOptions {
            count: Some(4),
            density: 1.5,
        };
        let with_hg = suggest_keyframes(&luma, Some(&hg), &opts).unwrap();
        assert_eq!(with_hg, vec![0, 49], "staircase should be cancelled");

        // Without compensation the jump reads as scene change and attracts
        // the interior keyframes.
        let without = suggest_keyframes(&luma, None, &opts).unwrap();
        assert!(without.len() > 2);
        assert!(
            without[1..without.len() - 1]
                .iter()
                .all(|&f| (15..=35).contains(&f)),
            "interior keyframes should cluster at the jump: {without:?}"
        );
    }

    #[test]
    fn too_few_frames_error() {
        assert!(suggest_keyframes(&[0.5], None, &SuggestOptions::default()).is_err());
    }
}
