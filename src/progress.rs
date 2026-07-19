use std::path::PathBuf;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;

/// Progress events emitted during a render. In `--progress json` mode each
/// event is one NDJSON line on stdout — the wire protocol for UIs driving
/// the CLI. Everything human-readable goes to stderr instead.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProgressEvent {
    Start {
        total_frames: usize,
        width: u32,
        height: u32,
    },
    Frame {
        index: usize,
        done: usize,
        total: usize,
    },
    /// Per-frame luminance measurement from an analysis pass.
    Luma {
        frame: usize,
        value: f32,
        done: usize,
        total: usize,
    },
    /// Suggested keyframe positions from the keyframe wizard.
    KeyframeSuggestion {
        frames: Vec<u32>,
    },
    /// One completed deflicker correction pass.
    DeflickerPass {
        pass: u32,
        frames_corrected: usize,
        max_delta_ev: f32,
    },
    Done {
        output: PathBuf,
        elapsed_ms: u64,
    },
    Warning {
        message: String,
    },
}

pub enum ProgressReporter {
    Human(ProgressBar),
    Json,
    /// Deliver events to an arbitrary callback — how embedding applications
    /// (a GUI, tests) receive progress without touching a terminal.
    Callback(Box<dyn Fn(ProgressEvent) + Send + Sync>),
}

impl ProgressReporter {
    pub fn callback(f: impl Fn(ProgressEvent) + Send + Sync + 'static) -> Self {
        Self::Callback(Box::new(f))
    }

    pub fn human() -> Self {
        let bar = ProgressBar::hidden();
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} frames",
            )
            .unwrap()
            .progress_chars("#>-"),
        );
        Self::Human(bar)
    }

    pub fn json() -> Self {
        Self::Json
    }

    pub fn report(&self, event: ProgressEvent) {
        match self {
            Self::Human(bar) => match &event {
                ProgressEvent::Start { total_frames, .. } => {
                    bar.set_length(*total_frames as u64);
                    bar.set_draw_target(indicatif::ProgressDrawTarget::stderr());
                    bar.enable_steady_tick(Duration::from_millis(100));
                }
                ProgressEvent::Frame { done, .. } | ProgressEvent::Luma { done, .. } => {
                    bar.set_position(*done as u64);
                }
                ProgressEvent::Done { output, elapsed_ms } => {
                    bar.finish_and_clear();
                    eprintln!(
                        "Done: {} ({:.2}s)",
                        output.display(),
                        *elapsed_ms as f64 / 1000.0
                    );
                }
                ProgressEvent::KeyframeSuggestion { frames } => {
                    bar.suspend(|| eprintln!("Suggested keyframes at frames: {frames:?}"));
                }
                ProgressEvent::DeflickerPass {
                    pass,
                    frames_corrected,
                    max_delta_ev,
                } => {
                    bar.suspend(|| {
                        eprintln!(
                            "Pass {pass}: corrected {frames_corrected} frame(s), max delta {max_delta_ev:.3} EV"
                        )
                    });
                    bar.set_position(0);
                }
                ProgressEvent::Warning { message } => {
                    bar.suspend(|| eprintln!("Warning: {message}"));
                }
            },
            Self::Json => {
                // One NDJSON line per event on stdout.
                if let Ok(line) = serde_json::to_string(&event) {
                    println!("{line}");
                }
            }
            Self::Callback(f) => f(event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_as_tagged_json() {
        let event = ProgressEvent::Frame {
            index: 3,
            done: 4,
            total: 10,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"event":"frame","index":3,"done":4,"total":10}"#);

        let start = ProgressEvent::Start {
            total_frames: 5,
            width: 100,
            height: 50,
        };
        assert!(serde_json::to_string(&start)
            .unwrap()
            .contains(r#""event":"start""#));
    }
}
