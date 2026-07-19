//! Background work: preview rendering and long-running engine jobs, with
//! results delivered back to the UI thread through a channel.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use lapsify::progress::{ProgressEvent, ProgressReporter};
use lapsify::project::Project;

/// What a finished job hands back to the UI.
// A few of these per second at most; variant size imbalance is irrelevant.
#[allow(clippy::large_enum_variant)]
pub enum UiEvent {
    PreviewReady {
        generation: u64,
        image: Box<egui::ColorImage>,
    },
    PreviewFailed {
        error: String,
    },
    Progress(ProgressEvent),
    /// A job finished; when it produced an updated project (analysis passes
    /// write layers), it rides along to replace the document's project.
    JobFinished {
        name: &'static str,
        result: Result<Option<Project>, String>,
    },
}

pub struct Worker {
    pub tx: Sender<UiEvent>,
    pub rx: Receiver<UiEvent>,
    egui_ctx: egui::Context,
    /// Monotonic id for preview requests: stale results are dropped.
    pub preview_generation: u64,
    preview_in_flight: bool,
    preview_pending: Option<PreviewRequest>,
    pub job_running: Option<&'static str>,
}

#[derive(Clone)]
pub struct PreviewRequest {
    pub project: Project,
    pub frame: u32,
    pub max_dim: u32,
}

impl Worker {
    pub fn new(egui_ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            egui_ctx,
            preview_generation: 0,
            preview_in_flight: false,
            preview_pending: None,
            job_running: None,
        }
    }

    /// Ask for a preview render. Coalesces: while one render is in flight,
    /// only the most recent request is remembered.
    pub fn request_preview(&mut self, request: PreviewRequest) {
        if self.preview_in_flight {
            self.preview_pending = Some(request);
            return;
        }
        self.spawn_preview(request);
    }

    fn spawn_preview(&mut self, request: PreviewRequest) {
        self.preview_generation += 1;
        let generation = self.preview_generation;
        self.preview_in_flight = true;

        let tx = self.tx.clone();
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let result =
                lapsify::render_preview(&request.project, request.frame, Some(request.max_dim));
            let event = match result {
                Ok(img) => {
                    let rgba = img.into_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let image = Box::new(egui::ColorImage::from_rgba_unmultiplied(
                        size,
                        rgba.as_raw(),
                    ));
                    UiEvent::PreviewReady { generation, image }
                }
                Err(e) => UiEvent::PreviewFailed {
                    error: e.to_string(),
                },
            };
            let _ = tx.send(event);
            ctx.request_repaint();
        });
    }

    /// Call when a PreviewReady/PreviewFailed arrives: kicks the coalesced
    /// follow-up render if the user moved on while we were rendering.
    pub fn preview_finished(&mut self) {
        self.preview_in_flight = false;
        if let Some(pending) = self.preview_pending.take() {
            self.spawn_preview(pending);
        }
    }

    /// Run an engine job on a worker thread. `f` receives a progress
    /// reporter that forwards events to the UI; returning `Some(project)`
    /// replaces the document's project (analysis passes).
    pub fn run_job(
        &mut self,
        name: &'static str,
        f: impl FnOnce(&ProgressReporter) -> Result<Option<Project>, String> + Send + 'static,
    ) {
        if self.job_running.is_some() {
            return;
        }
        self.job_running = Some(name);

        let tx = self.tx.clone();
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let reporter = ProgressReporter::callback(move |event| {
                let _ = progress_tx.send(UiEvent::Progress(event));
                progress_ctx.request_repaint();
            });

            let result = f(&reporter);
            let _ = tx.send(UiEvent::JobFinished { name, result });
            ctx.request_repaint();
        });
    }
}

/// Convenience: the frame list for a project, as the engine sees it.
pub fn frames_of(project: &Project) -> Result<Vec<PathBuf>, String> {
    lapsify::source::list_images(&project.input).map_err(|e| e.to_string())
}
