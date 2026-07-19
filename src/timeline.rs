//! Frame-to-position mapping for curve sampling.
//!
//! Color curves can interpolate in capture-time space (from EXIF timestamps)
//! instead of frame-index space, which matters for irregular shooting
//! intervals. Geometry (the crop track) intentionally stays in frame space:
//! motion is perceived in playback time, where frames are equally spaced.

use crate::project::{InterpolationMode, Project};

pub struct Timeline<'a> {
    times: Option<&'a [i64]>,
}

impl<'a> Timeline<'a> {
    /// The timeline a project's color curves should sample with. Falls back
    /// to frame indices when time interpolation is off, timestamps are
    /// missing, or timestamps are not strictly increasing.
    pub fn of(project: &'a Project) -> Self {
        let times = match project.interpolation {
            InterpolationMode::Frame => None,
            InterpolationMode::Time => project
                .analysis
                .as_ref()
                .and_then(|a| a.capture_times_ms.as_deref())
                .filter(|t| t.len() >= 2 && t.windows(2).all(|w| w[1] > w[0])),
        };
        Self { times }
    }

    /// Whether capture times are actually driving interpolation.
    pub fn is_time_based(&self) -> bool {
        self.times.is_some()
    }

    /// Position of a frame on the sampling axis.
    pub fn x(&self, frame: u32) -> f32 {
        match self.times {
            Some(times) => {
                let i = (frame as usize).min(times.len() - 1);
                (times[i] - times[0]) as f32
            }
            None => frame as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::curve::{Curve, Keyframe};
    use crate::project::{ColorGrade, ExportSettings, PROJECT_VERSION};
    use approx::assert_relative_eq;
    use std::path::PathBuf;

    fn project_with_times(times: Option<Vec<i64>>, mode: InterpolationMode) -> Project {
        Project {
            version: PROJECT_VERSION,
            input: PathBuf::from("frames"),
            frame_range: None,
            interpolation: mode,
            color: ColorGrade::default(),
            crop: None,
            export: ExportSettings::new(PathBuf::from("out")),
            analysis: times.map(|capture_times_ms| Analysis {
                capture_times_ms: Some(capture_times_ms),
                ..Analysis::default()
            }),
        }
    }

    #[test]
    fn frame_mode_ignores_times() {
        let project = project_with_times(Some(vec![0, 1000, 9000]), InterpolationMode::Frame);
        let timeline = Timeline::of(&project);
        assert!(!timeline.is_time_based());
        assert_relative_eq!(timeline.x(1), 1.0);
    }

    #[test]
    fn time_mode_uses_capture_spacing() {
        // Frames at 0s, 1s and 9s: frame 1 sits at 1/9 of the clip, not 1/2.
        let project = project_with_times(Some(vec![0, 1000, 9000]), InterpolationMode::Time);
        let timeline = Timeline::of(&project);
        assert!(timeline.is_time_based());

        let curve = Curve::Keyframed(vec![Keyframe::new(0, 0.0), Keyframe::new(2, 9.0)]);
        let frame_based = curve.sample(1);
        let time_based = curve.sample_mapped(1, |f| timeline.x(f));

        assert_relative_eq!(frame_based, 4.5, epsilon = 0.01);
        assert!(time_based < 2.0, "expected early value, got {time_based}");
    }

    #[test]
    fn degrades_on_bad_timestamps() {
        // Not strictly increasing -> frame fallback.
        let project = project_with_times(Some(vec![0, 5000, 5000]), InterpolationMode::Time);
        assert!(!Timeline::of(&project).is_time_based());

        let project = project_with_times(None, InterpolationMode::Time);
        assert!(!Timeline::of(&project).is_time_based());
    }
}
