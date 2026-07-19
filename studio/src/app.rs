use std::path::Path;

use lapsify::analysis::deflicker::{run_deflicker, DeflickerOptions};
use lapsify::analysis::holygrail::{compute_holy_grail, HolyGrailOptions};
use lapsify::analysis::keyframes::{suggest_keyframes, SuggestOptions};
use lapsify::analysis::luminance::{measure_luminance, LuminanceOptions};
use lapsify::analysis::Analysis;
use lapsify::curve::Keyframe;
use lapsify::progress::ProgressEvent;
use lapsify::Curve;

use crate::document::{Document, ParamId};
use crate::panels;
use crate::worker::{frames_of, PreviewRequest, UiEvent, Worker};

pub struct StudioApp {
    pub doc: Option<Document>,
    pub worker: Worker,

    // Playback / navigation
    pub current_frame: u32,

    // Preview state
    pub preview_texture: Option<egui::TextureHandle>,
    pub preview_generation_shown: u64,
    pub preview_dirty: bool,

    // Job progress
    pub progress: Option<(usize, usize)>, // done, total
    pub deflicker_note: Option<String>,
    pub status: String,
    pub error: Option<String>,

    // Curve panel state
    pub selected_param: ParamId,
    pub dragging_keyframe: Option<usize>,
    pub show_source_luma: bool,
    pub show_developed_luma: bool,
    pub show_compensation: bool,
    pub show_deflicker: bool,

    // Branding
    pub logo: Option<egui::TextureHandle>,

    // Debug self-test state
    selftest_started: bool,
}

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::theme::apply(&cc.egui_ctx);
        let logo = image::load_from_memory(include_bytes!("../assets/mark.png"))
            .ok()
            .map(|img| {
                let rgba = img.into_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                cc.egui_ctx.load_texture(
                    "logo",
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                    egui::TextureOptions::LINEAR,
                )
            });
        Self {
            doc: None,
            worker: Worker::new(cc.egui_ctx.clone()),
            current_frame: 0,
            preview_texture: None,
            preview_generation_shown: 0,
            preview_dirty: false,
            progress: None,
            deflicker_note: None,
            status: "Open a frames folder or a project.json to start".to_string(),
            error: None,
            selected_param: ParamId::Exposure,
            dragging_keyframe: None,
            show_source_luma: true,
            show_developed_luma: false,
            show_compensation: false,
            show_deflicker: false,
            logo,
            selftest_started: false,
        }
    }

    pub fn open_folder(&mut self, dir: &Path) {
        match Document::open_folder(dir) {
            Ok(doc) => self.install_document(doc),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn open_project(&mut self, path: &Path) {
        match Document::open_project(path) {
            Ok(doc) => self.install_document(doc),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn install_document(&mut self, doc: Document) {
        self.status = format!(
            "{} — {} frames",
            doc.project.input.display(),
            doc.frame_count()
        );
        self.current_frame = 0;
        self.preview_texture = None;
        self.error = None;
        self.doc = Some(doc);
        self.preview_dirty = true;
    }

    pub fn request_preview(&mut self) {
        if let Some(doc) = &self.doc {
            self.worker.request_preview(PreviewRequest {
                project: doc.project.clone(),
                frame: self.current_frame,
                max_dim: 1400,
            });
            self.preview_dirty = false;
        }
    }

    fn pump_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.worker.rx.try_recv() {
            match event {
                UiEvent::PreviewReady { generation, image } => {
                    self.worker.preview_finished();
                    if generation > self.preview_generation_shown {
                        self.preview_generation_shown = generation;
                        self.preview_texture =
                            Some(ctx.load_texture("preview", *image, egui::TextureOptions::LINEAR));
                    }
                }
                UiEvent::PreviewFailed { error, .. } => {
                    self.worker.preview_finished();
                    self.error = Some(error);
                }
                UiEvent::Progress(progress) => match progress {
                    ProgressEvent::Start { total_frames, .. } => {
                        self.progress = Some((0, total_frames));
                    }
                    ProgressEvent::Frame { done, total, .. }
                    | ProgressEvent::Luma { done, total, .. } => {
                        self.progress = Some((done, total));
                    }
                    ProgressEvent::DeflickerPass {
                        pass,
                        frames_corrected,
                        max_delta_ev,
                    } => {
                        self.deflicker_note = Some(format!(
                            "pass {pass}: {frames_corrected} corrected, max Δ {max_delta_ev:.3} EV"
                        ));
                    }
                    ProgressEvent::Done { .. } => {}
                    ProgressEvent::Warning { message } => {
                        self.status = format!("Warning: {message}");
                    }
                    ProgressEvent::KeyframeSuggestion { frames } => {
                        self.status = format!("Suggested keyframes: {frames:?}");
                    }
                },
                UiEvent::JobFinished { name, result } => {
                    self.worker.job_running = None;
                    self.progress = None;
                    match result {
                        Ok(updated) => {
                            if let (Some(doc), Some(project)) = (&mut self.doc, updated) {
                                doc.project = project;
                                doc.dirty = true;
                                self.preview_dirty = true;
                            }
                            self.status = format!("{name} finished");
                        }
                        Err(e) => self.error = Some(format!("{name}: {e}")),
                    }
                }
            }
        }
    }

    // ----- engine jobs -------------------------------------------------

    pub fn job_luminance(&mut self, developed: bool) {
        let Some(doc) = &self.doc else { return };
        let project = doc.project.clone();
        let name = if developed {
            "developed luminance"
        } else {
            "luminance analysis"
        };
        self.worker.run_job(name, move |reporter| {
            let frames = frames_of(&project)?;
            let opts = LuminanceOptions {
                developed,
                ..Default::default()
            };
            let series =
                measure_luminance(&project, &frames, &opts, reporter).map_err(|e| e.to_string())?;
            let mut project = project;
            let analysis = project.analysis.get_or_insert_with(Analysis::default);
            if developed {
                analysis.developed_luminance = Some(series);
            } else {
                analysis.source_luminance = Some(series);
            }
            Ok(Some(project))
        });
    }

    pub fn job_holygrail(&mut self) {
        let Some(doc) = &self.doc else { return };
        let project = doc.project.clone();
        self.worker
            .run_job("exposure compensation", move |reporter| {
                let frames = frames_of(&project)?;
                let opts = HolyGrailOptions {
                    rotate: None,
                    stretch: None,
                };
                let (layer, times) =
                    compute_holy_grail(&frames, &opts, reporter).map_err(|e| e.to_string())?;
                let mut project = project;
                let analysis = project.analysis.get_or_insert_with(Analysis::default);
                analysis.holy_grail = Some(layer);
                if times.is_some() {
                    analysis.capture_times_ms = times;
                }
                Ok(Some(project))
            });
    }

    pub fn job_deflicker(&mut self) {
        let Some(doc) = &self.doc else { return };
        let project = doc.project.clone();
        self.deflicker_note = None;
        self.worker.run_job("deflicker", move |reporter| {
            let frames = frames_of(&project)?;
            let layer = run_deflicker(&project, &frames, &DeflickerOptions::default(), reporter)
                .map_err(|e| e.to_string())?;
            let mut project = project;
            let analysis = project.analysis.get_or_insert_with(Analysis::default);
            analysis.deflicker = Some(layer);
            Ok(Some(project))
        });
    }

    pub fn job_suggest_keyframes(&mut self) {
        let Some(doc) = &self.doc else { return };
        let project = doc.project.clone();
        self.worker.run_job("keyframe suggestion", move |reporter| {
            let analysis = project.analysis.as_ref();
            let luma = analysis
                .and_then(|a| a.source_luminance.as_ref())
                .ok_or("Run luminance analysis first")?;
            let frames = suggest_keyframes(
                &luma.values,
                analysis.and_then(|a| a.holy_grail.as_ref()),
                &SuggestOptions::default(),
            )
            .map_err(|e| e.to_string())?;
            reporter.report(ProgressEvent::KeyframeSuggestion {
                frames: frames.clone(),
            });

            // Insert no-op handles at the suggested frames.
            let mut project = project;
            let mut keyframes: Vec<Keyframe> = match &project.color.exposure {
                Curve::Keyframed(kfs) => kfs.clone(),
                Curve::Constant(_) => Vec::new(),
            };
            for &frame in &frames {
                if keyframes.iter().any(|k| k.frame.abs_diff(frame) <= 2) {
                    continue;
                }
                keyframes.push(Keyframe::new(frame, project.color.exposure.sample(frame)));
            }
            keyframes.sort_by_key(|k| k.frame);
            project.color.exposure = Curve::Keyframed(keyframes);
            Ok(Some(project))
        });
    }

    pub fn job_render(&mut self) {
        let Some(doc) = &self.doc else { return };
        let project = doc.project.clone();
        self.worker.run_job("render", move |reporter| {
            let start = std::time::Instant::now();
            if project.is_video_output() {
                lapsify::export::video::render_to_video(&project, reporter, start)
                    .map_err(|e| e.to_string())?;
            } else {
                lapsify::export::images::render_to_images(&project, reporter, start)
                    .map_err(|e| e.to_string())?;
            }
            Ok(None)
        });
    }

    pub fn save(&mut self) {
        if let Some(doc) = &mut self.doc {
            match doc.save() {
                Ok(path) => self.status = format!("Saved {}", path.display()),
                Err(e) => self.error = Some(e.to_string()),
            }
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events(ctx);

        panels::toolbar::show(self, ctx);
        panels::adjustments::show(self, ctx);
        panels::timeline::show(self, ctx);
        panels::preview::show(self, ctx);

        if self.preview_dirty {
            self.request_preview();
        }

        self.debug_self_shot(ctx);
    }
}

impl StudioApp {
    /// Debug/CI hook: with LAPSIFY_STUDIO_SHOT=/path.png set, capture the
    /// window once the preview has settled, write it, and exit. With
    /// LAPSIFY_STUDIO_SELFTEST=1 as well, a luminance analysis runs first so
    /// the whole worker pipeline is exercised before the capture.
    fn debug_self_shot(&mut self, ctx: &egui::Context) {
        let Ok(path) = std::env::var("LAPSIFY_STUDIO_SHOT") else {
            return;
        };

        if std::env::var("LAPSIFY_STUDIO_SELFTEST").is_ok()
            && !self.selftest_started
            && self.doc.is_some()
        {
            self.selftest_started = true;
            self.job_luminance(false);
            return;
        }

        // Save the screenshot when it arrives.
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let size = [image.size[0] as u32, image.size[1] as u32];
            eprintln!("self-shot: got {}x{} screenshot", size[0], size[1]);
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            match image::RgbaImage::from_raw(size[0], size[1], rgba) {
                Some(buffer) => match buffer.save(&path) {
                    Ok(()) => eprintln!("self-shot: saved {path}"),
                    Err(e) => eprintln!("self-shot: save failed: {e}"),
                },
                None => eprintln!("self-shot: buffer size mismatch"),
            }
            std::process::exit(0);
        }

        // Trigger once things settle: preview rendered and no job running.
        let settled = self.doc.is_none()
            || (self.preview_texture.is_some() && self.worker.job_running.is_none());
        if settled {
            ctx.request_repaint();
            // Give the UI a few frames to lay out before capturing.
            let frames = ctx.cumulative_frame_nr();
            if frames > 20 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
        }
    }
}
