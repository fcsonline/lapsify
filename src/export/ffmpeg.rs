use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::JoinHandle;

use image::RgbImage;

use crate::error::{LapsifyError, Result};
use crate::export::{parse_resolution, FrameSink};
use crate::project::{Codec, Project};

/// Encodes frames by piping raw RGB24 to ffmpeg's stdin: no intermediate
/// files, no double compression, exact frame framing by byte count.
pub struct FfmpegSink {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr_thread: Option<JoinHandle<String>>,
    output: PathBuf,
    width: u32,
    height: u32,
}

impl FfmpegSink {
    pub fn spawn(project: &Project, width: u32, height: u32) -> Result<(Self, PathBuf)> {
        let export = &project.export;
        let output = export.output.join(format!("timelapse.{}", export.format));

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-hide_banner")
            .arg("-loglevel")
            .arg("error")
            .arg("-y")
            // input: raw frames on stdin
            .arg("-f")
            .arg("rawvideo")
            .arg("-pix_fmt")
            .arg("rgb24")
            .arg("-s")
            .arg(format!("{width}x{height}"))
            .arg("-framerate")
            .arg(export.fps.to_string())
            .arg("-i")
            .arg("-");

        // Scale to the requested resolution (fitting within it, preserving
        // aspect ratio), then force even dimensions for chroma subsampling.
        let filter = match export.resolution.as_deref() {
            Some(res) => {
                let (w, h) = parse_resolution(res)?;
                format!(
                    "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos,scale=trunc(iw/2)*2:trunc(ih/2)*2"
                )
            }
            None => "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string(),
        };
        cmd.arg("-vf").arg(filter);

        match export.codec {
            Codec::H264 => {
                cmd.arg("-c:v")
                    .arg("libx264")
                    .arg("-preset")
                    .arg("medium")
                    .arg("-crf")
                    .arg(export.quality.to_string())
                    .arg("-pix_fmt")
                    .arg("yuv420p");
            }
            Codec::H265 => {
                cmd.arg("-c:v")
                    .arg("libx265")
                    .arg("-tag:v")
                    .arg("hvc1")
                    .arg("-crf")
                    .arg(export.quality.to_string())
                    .arg("-pix_fmt")
                    .arg(if export.ten_bit {
                        "yuv420p10le"
                    } else {
                        "yuv420p"
                    });
            }
            Codec::Prores => {
                cmd.arg("-c:v")
                    .arg("prores_ks")
                    .arg("-profile:v")
                    .arg("3")
                    .arg("-qscale:v")
                    .arg("9")
                    .arg("-pix_fmt")
                    .arg("yuv422p10le");
            }
        }

        if export.format == "mp4" || export.format == "mov" {
            cmd.arg("-movflags").arg("+faststart");
        }

        cmd.arg(&output)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            LapsifyError::message(format!(
                "Failed to start ffmpeg (is it installed and on PATH?): {e}"
            ))
        })?;

        let stdin = child.stdin.take();

        // Drain stderr on its own thread to avoid pipe deadlock; keep the tail
        // for error reporting.
        let mut stderr = child.stderr.take().expect("stderr piped");
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stderr.read_to_string(&mut buf);
            let skip = buf.chars().count().saturating_sub(2000);
            buf.chars().skip(skip).collect()
        });

        Ok((
            Self {
                child,
                stdin,
                stderr_thread: Some(stderr_thread),
                output: output.clone(),
                width,
                height,
            },
            output,
        ))
    }

    fn fail(&mut self, context: &str) -> LapsifyError {
        let _ = self.child.kill();
        let code = self.child.wait().ok().and_then(|s| s.code());
        let stderr_tail = self
            .stderr_thread
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        LapsifyError::Ffmpeg {
            code,
            stderr_tail: format!("{context}: {stderr_tail}"),
        }
    }
}

impl FrameSink for FfmpegSink {
    fn write_frame(&mut self, index: usize, frame: &RgbImage) -> Result<()> {
        if frame.dimensions() != (self.width, self.height) {
            return Err(LapsifyError::message(format!(
                "frame {index} is {}x{}, but the encoder expects {}x{}",
                frame.width(),
                frame.height(),
                self.width,
                self.height
            )));
        }
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| LapsifyError::message("ffmpeg stdin already closed"))?;
        if let Err(e) = stdin.write_all(frame.as_raw()) {
            return Err(self.fail(&format!("writing frame {index} failed ({e})")));
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<()> {
        // Closing stdin signals end of stream.
        drop(self.stdin.take());

        let status = self
            .child
            .wait()
            .map_err(|e| LapsifyError::message(format!("waiting for ffmpeg failed: {e}")))?;

        let stderr_tail = self
            .stderr_thread
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default();

        if !status.success() {
            return Err(LapsifyError::Ffmpeg {
                code: status.code(),
                stderr_tail,
            });
        }

        if !self.output.exists() {
            return Err(LapsifyError::message(format!(
                "ffmpeg exited successfully but {} was not created",
                self.output.display()
            )));
        }

        Ok(())
    }
}
