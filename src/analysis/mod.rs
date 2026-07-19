//! Machine-generated analysis state.
//!
//! Everything here is derived data that analysis commands write back into the
//! project JSON. Each layer records the fingerprint of the source folder it
//! was computed from, so stale analysis is detectable, and re-running any
//! analysis command overwrites its layer wholesale (idempotent by
//! construction).

pub mod deflicker;
pub mod holygrail;
pub mod keyframes;
pub mod luminance;

pub use deflicker::DeflickerLayer;
pub use holygrail::HolyGrailLayer;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh64::Xxh64;

use crate::crop::CropRect;
use crate::error::{LapsifyError, Result};

/// Analysis state stored inside the project JSON under "analysis".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Analysis {
    /// Mean linear luminance of the SOURCE frames (before any grading).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_luminance: Option<LumaSeries>,
    /// Mean linear luminance of the DEVELOPED frames (all grading applied).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub developed_luminance: Option<LumaSeries>,
    /// Exposure compensation for in-camera exposure changes, from EXIF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holy_grail: Option<HolyGrailLayer>,
    /// Per-frame exposure corrections from visual deflicker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deflicker: Option<DeflickerLayer>,
    /// Capture timestamps in unix epoch milliseconds, one per frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_times_ms: Option<Vec<i64>>,
}

/// A per-frame scalar luminance series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LumaSeries {
    /// Mean linear Rec.709 luminance per frame, one entry per source frame.
    pub values: Vec<f32>,
    /// Normalized source-image region the measurement was restricted to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<CropRect>,
    /// Long-edge size the frames were downscaled to before measuring.
    pub measure_dim: u32,
    /// Unix timestamp (seconds) of the measurement.
    pub computed_at_unix: u64,
    /// Fingerprint of the source folder at measurement time.
    pub source_fingerprint: String,
}

/// Fingerprint of a frame list: filename, size and mtime of every file.
/// Changes when frames are added, removed, renamed or rewritten.
pub fn source_fingerprint(image_files: &[PathBuf]) -> Result<String> {
    let mut hasher = Xxh64::new(0);
    for path in image_files {
        let meta = std::fs::metadata(path).map_err(|e| LapsifyError::io(path, e))?;
        if let Some(name) = path.file_name() {
            hasher.update(name.to_string_lossy().as_bytes());
        }
        hasher.update(&meta.len().to_le_bytes());
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        hasher.update(&mtime.to_le_bytes());
    }
    Ok(format!("{:016x}", hasher.digest()))
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fingerprint_changes_with_content() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.jpg");
        let b = tmp.path().join("b.jpg");
        fs::write(&a, b"one").unwrap();
        fs::write(&b, b"two").unwrap();

        let fp1 = source_fingerprint(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(fp1, source_fingerprint(&[a.clone(), b.clone()]).unwrap());

        fs::write(&b, b"two-changed").unwrap();
        let fp2 = source_fingerprint(&[a.clone(), b.clone()]).unwrap();
        assert_ne!(fp1, fp2);

        // Order matters through sorting upstream; same list = same hash.
        let fp3 = source_fingerprint(&[b, a]).unwrap();
        assert_ne!(fp2, fp3);
    }
}
