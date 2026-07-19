use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::crop::CropTrack;
use crate::curve::Curve;
use crate::error::{LapsifyError, Result};

pub const PROJECT_VERSION: u32 = 1;

/// A lapsify project: the single source of truth for a render. The CLI flags
/// build one of these, and a project JSON file deserializes into one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    /// Directory of source frames.
    pub input: PathBuf,
    /// Inclusive frame range to process, 0-based. None = all frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_range: Option<(usize, usize)>,
    #[serde(default)]
    pub color: ColorGrade,
    /// Crop window over time in normalized source-image coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<CropTrack>,
    pub export: ExportSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrade {
    /// Exposure in EV stops.
    pub exposure: Curve,
    /// Brightness offset, -100 to +100.
    pub brightness: Curve,
    /// Contrast multiplier around mid-gray.
    pub contrast: Curve,
    /// Saturation multiplier.
    pub saturation: Curve,
}

impl Default for ColorGrade {
    fn default() -> Self {
        Self {
            exposure: Curve::Constant(0.0),
            brightness: Curve::Constant(0.0),
            contrast: Curve::Constant(1.0),
            saturation: Curve::Constant(1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    #[default]
    H264,
    H265,
    Prores,
}

impl std::str::FromStr for Codec {
    type Err = LapsifyError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "h264" | "x264" | "avc" => Ok(Self::H264),
            "h265" | "x265" | "hevc" => Ok(Self::H265),
            "prores" => Ok(Self::Prores),
            other => Err(LapsifyError::message(format!(
                "Unknown codec '{other}' (expected h264, h265 or prores)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    /// Output directory.
    pub output: PathBuf,
    /// jpg, png, tiff for image sequences; mp4, mov, avi for video.
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// Video quality (CRF, 0-51, lower is better). Ignored by ProRes.
    #[serde(default = "default_quality")]
    pub quality: u32,
    /// Target video resolution, e.g. "1920x1080", "4K".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default)]
    pub codec: Codec,
    /// Encode with 10-bit chroma (h265 and prores only).
    #[serde(default)]
    pub ten_bit: bool,
    /// JPEG quality for image-sequence output (1-100).
    #[serde(default = "default_jpeg_quality")]
    pub jpeg_quality: u8,
}

fn default_format() -> String {
    "mp4".to_string()
}

fn default_fps() -> u32 {
    24
}

fn default_quality() -> u32 {
    20
}

fn default_jpeg_quality() -> u8 {
    90
}

impl Project {
    pub fn from_json(json: &str) -> Result<Self> {
        let project: Project = serde_json::from_str(json)
            .map_err(|e| LapsifyError::message(format!("Failed to parse project file: {e}")))?;
        if project.version != PROJECT_VERSION {
            return Err(LapsifyError::message(format!(
                "Unsupported project version {} (this build supports version {})",
                project.version, PROJECT_VERSION
            )));
        }
        Ok(project)
    }

    pub fn from_json_file(path: &Path) -> Result<Self> {
        let json = fs::read_to_string(path).map_err(|e| LapsifyError::io(path, e))?;
        Self::from_json(&json)
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| LapsifyError::message(format!("Failed to serialize project: {e}")))
    }

    pub fn is_video_output(&self) -> bool {
        matches!(self.export.format.as_str(), "mp4" | "mov" | "avi")
    }

    /// Validate curve structure and parameter ranges.
    pub fn validate(&self) -> Result<()> {
        self.color.exposure.validate("exposure")?;
        self.color.brightness.validate("brightness")?;
        self.color.contrast.validate("contrast")?;
        self.color.saturation.validate("saturation")?;

        self.color.exposure.validate_range("exposure", -3.0, 3.0)?;
        self.color
            .brightness
            .validate_range("brightness", -100.0, 100.0)?;
        self.color.contrast.validate_range("contrast", 0.1, 3.0)?;
        self.color
            .saturation
            .validate_range("saturation", 0.0, 2.0)?;

        if let Some(ref crop) = self.crop {
            crop.validate()?;
        }

        if !(1..=120).contains(&self.export.fps) {
            return Err(LapsifyError::message("FPS must be between 1 and 120"));
        }
        if self.export.quality > 51 {
            return Err(LapsifyError::message(
                "Quality (CRF) must be between 0 and 51",
            ));
        }
        if let Some((start, end)) = self.frame_range {
            if start > end {
                return Err(LapsifyError::message(
                    "Start frame must be less than or equal to end frame",
                ));
            }
        }
        if !(1..=100).contains(&self.export.jpeg_quality) {
            return Err(LapsifyError::message(
                "JPEG quality must be between 1 and 100",
            ));
        }
        if self.export.ten_bit && self.export.codec == Codec::H264 {
            return Err(LapsifyError::message(
                "10-bit output requires the h265 or prores codec",
            ));
        }
        if self.export.codec == Codec::Prores && self.export.format != "mov" {
            return Err(LapsifyError::message(
                "ProRes requires the mov container (use -f mov)",
            ));
        }

        Ok(())
    }
}

impl ExportSettings {
    pub fn new(output: PathBuf) -> Self {
        Self {
            output,
            format: default_format(),
            fps: default_fps(),
            quality: default_quality(),
            resolution: None,
            codec: Codec::default(),
            ten_bit: false,
            jpeg_quality: default_jpeg_quality(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Keyframe;

    fn minimal_project() -> Project {
        Project {
            version: PROJECT_VERSION,
            input: PathBuf::from("frames"),
            frame_range: None,
            color: ColorGrade::default(),
            crop: None,
            export: ExportSettings::new(PathBuf::from("out")),
        }
    }

    #[test]
    fn json_roundtrip() {
        let mut project = minimal_project();
        project.color.exposure =
            Curve::Keyframed(vec![Keyframe::new(0, 0.0), Keyframe::new(120, 1.5)]);

        let json = project.to_json_pretty().unwrap();
        let parsed = Project::from_json(&json).unwrap();
        assert_eq!(parsed.color.exposure, project.color.exposure);
        assert_eq!(parsed.input, project.input);
    }

    #[test]
    fn parses_minimal_json_with_defaults() {
        let json = r#"{
            "version": 1,
            "input": "frames",
            "export": { "output": "out" }
        }"#;
        let project = Project::from_json(json).unwrap();
        assert_eq!(project.export.format, "mp4");
        assert_eq!(project.export.fps, 24);
        assert_eq!(project.color.contrast, Curve::Constant(1.0));
        assert!(project.validate().is_ok());
    }

    #[test]
    fn rejects_unknown_version() {
        let json = r#"{ "version": 99, "input": "frames", "export": { "output": "out" } }"#;
        assert!(Project::from_json(json).is_err());
    }

    #[test]
    fn validate_rejects_out_of_range() {
        let mut project = minimal_project();
        project.color.exposure = Curve::Constant(10.0);
        assert!(project.validate().is_err());

        let mut project = minimal_project();
        project.export.fps = 500;
        assert!(project.validate().is_err());

        let mut project = minimal_project();
        project.frame_range = Some((10, 5));
        assert!(project.validate().is_err());
    }
}
