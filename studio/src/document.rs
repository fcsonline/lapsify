//! The document: a lapsify `Project` plus everything the editor needs to
//! work with it (file path, frame list, dirty flag, keyframe-aware edits).

use std::path::{Path, PathBuf};

use lapsify::curve::{Easing, Keyframe};
use lapsify::error::Result;
use lapsify::project::{ExportSettings, Project, PROJECT_VERSION};
use lapsify::source::list_images;
use lapsify::Curve;

/// Identifies one keyframable color parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamId {
    Exposure,
    Temperature,
    Tint,
    Brightness,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Gamma,
    Saturation,
    Vibrance,
}

impl ParamId {
    pub const ALL: [ParamId; 12] = [
        ParamId::Exposure,
        ParamId::Temperature,
        ParamId::Tint,
        ParamId::Brightness,
        ParamId::Contrast,
        ParamId::Highlights,
        ParamId::Shadows,
        ParamId::Whites,
        ParamId::Blacks,
        ParamId::Gamma,
        ParamId::Saturation,
        ParamId::Vibrance,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ParamId::Exposure => "Exposure (EV)",
            ParamId::Temperature => "Temperature",
            ParamId::Tint => "Tint",
            ParamId::Brightness => "Brightness",
            ParamId::Contrast => "Contrast",
            ParamId::Highlights => "Highlights",
            ParamId::Shadows => "Shadows",
            ParamId::Whites => "Whites",
            ParamId::Blacks => "Blacks",
            ParamId::Gamma => "Gamma",
            ParamId::Saturation => "Saturation",
            ParamId::Vibrance => "Vibrance",
        }
    }

    pub fn range(&self) -> (f32, f32) {
        match self {
            ParamId::Exposure => (-3.0, 3.0),
            ParamId::Contrast => (0.1, 3.0),
            ParamId::Gamma => (0.2, 5.0),
            ParamId::Saturation => (0.0, 2.0),
            _ => (-100.0, 100.0),
        }
    }

    pub fn neutral(&self) -> f32 {
        match self {
            ParamId::Contrast | ParamId::Gamma | ParamId::Saturation => 1.0,
            _ => 0.0,
        }
    }
}

pub struct Document {
    pub project: Project,
    /// Where the project JSON lives (assigned on first save if opened from a
    /// bare folder).
    pub path: Option<PathBuf>,
    pub frames: Vec<PathBuf>,
    pub dirty: bool,
}

impl Document {
    pub fn open_project(path: &Path) -> Result<Self> {
        let project = Project::from_json_file(path)?;
        let frames = list_images(&project.input)?;
        Ok(Self {
            project,
            path: Some(path.to_path_buf()),
            frames,
            dirty: false,
        })
    }

    pub fn open_folder(dir: &Path) -> Result<Self> {
        let frames = list_images(dir)?;
        // Keep generated files out of the frame folder (images written there
        // would be picked up as source frames on the next scan).
        let output = dir.parent().unwrap_or(dir).join(format!(
            "{}-out",
            dir.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ));
        let project = Project {
            version: PROJECT_VERSION,
            input: dir.to_path_buf(),
            frame_range: None,
            interpolation: Default::default(),
            color: Default::default(),
            crop: None,
            export: ExportSettings::new(output),
            analysis: None,
        };
        Ok(Self {
            project,
            path: None,
            frames,
            dirty: true,
        })
    }

    /// Default project path when the document was opened from a bare folder.
    pub fn default_project_path(&self) -> PathBuf {
        self.project.input.join("project.json")
    }

    pub fn save(&mut self) -> Result<PathBuf> {
        let path = self
            .path
            .clone()
            .unwrap_or_else(|| self.default_project_path());
        self.project.save_atomic(&path)?;
        self.path = Some(path.clone());
        self.dirty = false;
        Ok(path)
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn curve(&self, param: ParamId) -> &Curve {
        let c = &self.project.color;
        match param {
            ParamId::Exposure => &c.exposure,
            ParamId::Temperature => &c.temperature,
            ParamId::Tint => &c.tint,
            ParamId::Brightness => &c.brightness,
            ParamId::Contrast => &c.contrast,
            ParamId::Highlights => &c.highlights,
            ParamId::Shadows => &c.shadows,
            ParamId::Whites => &c.whites,
            ParamId::Blacks => &c.blacks,
            ParamId::Gamma => &c.gamma,
            ParamId::Saturation => &c.saturation,
            ParamId::Vibrance => &c.vibrance,
        }
    }

    pub fn curve_mut(&mut self, param: ParamId) -> &mut Curve {
        let c = &mut self.project.color;
        match param {
            ParamId::Exposure => &mut c.exposure,
            ParamId::Temperature => &mut c.temperature,
            ParamId::Tint => &mut c.tint,
            ParamId::Brightness => &mut c.brightness,
            ParamId::Contrast => &mut c.contrast,
            ParamId::Highlights => &mut c.highlights,
            ParamId::Shadows => &mut c.shadows,
            ParamId::Whites => &mut c.whites,
            ParamId::Blacks => &mut c.blacks,
            ParamId::Gamma => &mut c.gamma,
            ParamId::Saturation => &mut c.saturation,
            ParamId::Vibrance => &mut c.vibrance,
        }
    }

    /// The parameter value at a frame.
    pub fn value_at(&self, param: ParamId, frame: u32) -> f32 {
        self.curve(param).sample(frame)
    }

    /// Whether the curve has a keyframe exactly at this frame.
    pub fn has_keyframe_at(&self, param: ParamId, frame: u32) -> bool {
        matches!(self.curve(param), Curve::Keyframed(kfs) if kfs.iter().any(|k| k.frame == frame))
    }

    /// Keyframe-aware edit: a constant curve stays constant (global change);
    /// a keyframed curve gets a keyframe written/updated at the frame.
    pub fn set_value(&mut self, param: ParamId, frame: u32, value: f32) {
        let curve = self.curve_mut(param);
        match curve {
            Curve::Constant(v) => *v = value,
            Curve::Keyframed(kfs) => match kfs.binary_search_by_key(&frame, |k| k.frame) {
                Ok(i) => kfs[i].value = value,
                Err(i) => kfs.insert(i, Keyframe::new(frame, value)),
            },
        }
        self.dirty = true;
    }

    /// Toggle a keyframe at the frame: converts a constant curve to a
    /// keyframed one, removes an existing keyframe (falling back to constant
    /// when it was the last), or adds a keyframe at the sampled value.
    pub fn toggle_keyframe(&mut self, param: ParamId, frame: u32) {
        let current = self.value_at(param, frame);
        let curve = self.curve_mut(param);
        match curve {
            Curve::Constant(_) => {
                *curve = Curve::Keyframed(vec![Keyframe {
                    frame,
                    value: current,
                    easing: Easing::Smooth,
                }]);
            }
            Curve::Keyframed(kfs) => match kfs.binary_search_by_key(&frame, |k| k.frame) {
                Ok(i) => {
                    kfs.remove(i);
                    if kfs.is_empty() {
                        *curve = Curve::Constant(current);
                    }
                }
                Err(i) => kfs.insert(i, Keyframe::new(frame, current)),
            },
        }
        self.dirty = true;
    }
}
