use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LapsifyError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Image(#[from] image::ImageError),

    #[error("invalid {field}: {reason}")]
    InvalidParam { field: &'static str, reason: String },

    #[error("frame {index} is {got_w}x{got_h}, expected {want_w}x{want_h} ({path})")]
    MixedFrameSizes {
        index: usize,
        path: PathBuf,
        got_w: u32,
        got_h: u32,
        want_w: u32,
        want_h: u32,
    },

    #[error("ffmpeg failed (exit {code:?}): {stderr_tail}")]
    Ffmpeg {
        code: Option<i32>,
        stderr_tail: String,
    },

    #[error("{0}")]
    Message(String),
}

impl LapsifyError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

impl From<String> for LapsifyError {
    fn from(msg: String) -> Self {
        Self::Message(msg)
    }
}

impl From<&str> for LapsifyError {
    fn from(msg: &str) -> Self {
        Self::Message(msg.to_string())
    }
}

pub type Result<T> = std::result::Result<T, LapsifyError>;
