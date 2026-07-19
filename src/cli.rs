use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::parser::ValueSource;
use clap::{Arg, ArgMatches, Command};
use colored::*;

use crate::crop::{legacy_crop_to_track, parse_crop_dims};
use crate::curve::{curve_from_legacy_array, parse_value_array, Curve};
use crate::error::{LapsifyError, Result};
use crate::export::images::render_to_images;
use crate::export::video::render_to_video;
use crate::progress::ProgressReporter;
use crate::project::{Codec, ColorGrade, ExportSettings, Project, PROJECT_VERSION};
use crate::source::{list_images, scan_dimensions};

fn build_command() -> Command {
    render_args(
        Command::new("lapsify")
            .version(env!("CARGO_PKG_VERSION"))
            .about("Process time-lapse images with keyframable adjustments"),
    )
    .subcommand(render_args(Command::new("render").about(
        "Render the full sequence to video or processed images",
    )))
    .subcommand(
        render_args(Command::new("preview").about("Render a single frame for inspection"))
            .arg(
                Arg::new("frame")
                    .long("frame")
                    .value_name("INDEX")
                    .help("Frame index to preview (0-based)")
                    .required(true),
            )
            .arg(
                Arg::new("max-dim")
                    .long("max-dim")
                    .value_name("PIXELS")
                    .help("Downscale the source so its longest edge fits this size before processing (faster previews)"),
            )
            .arg(
                Arg::new("out")
                    .long("out")
                    .value_name("FILE")
                    .help("Output image file")
                    .default_value("preview.png"),
            ),
    )
    .subcommand(
        Command::new("project")
            .about("Project file utilities")
            .subcommand_required(true)
            .subcommand(render_args(Command::new("dump").about(
                "Print the project JSON that the given flags produce",
            ))),
    )
}

/// The shared flag set accepted at the top level (legacy), by `render`, by
/// `preview` and by `project dump`.
fn render_args(cmd: Command) -> Command {
    cmd
        .arg(
            Arg::new("project")
                .short('p')
                .long("project")
                .value_name("FILE")
                .help("Project file (JSON) describing input, adjustments and export settings. Other flags override its values"),
        )
        .arg(
            Arg::new("input")
                .short('i')
                .long("input")
                .value_name("DIR")
                .help("Input directory containing images"),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("DIR")
                .help("Output directory for processed images"),
        )
        .arg(
            Arg::new("exposure")
                .allow_hyphen_values(true)
                .short('e')
                .long("exposure")
                .value_name("STOPS")
                .help("Exposure adjustment in EV stops. Single value (-3.0 to +3.0) or comma-separated array (e.g., '0.0,1.5,-0.5')")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("brightness")
                .allow_hyphen_values(true)
                .short('b')
                .long("brightness")
                .value_name("VALUE")
                .help("Brightness adjustment. Single value (-100 to +100) or comma-separated array (e.g., '0,20,-10')")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("contrast")
                .allow_hyphen_values(true)
                .short('c')
                .long("contrast")
                .value_name("VALUE")
                .help("Contrast multiplier. Single value (0.1 to 3.0) or comma-separated array (e.g., '1.0,1.5,0.8')")
                .default_value("1.0"),
        )
        .arg(
            Arg::new("saturation")
                .allow_hyphen_values(true)
                .short('s')
                .long("saturation")
                .value_name("VALUE")
                .help("Saturation multiplier. Single value (0.0 to 2.0) or comma-separated array (e.g., '1.0,1.8,0.5')")
                .default_value("1.0"),
        )
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .value_name("FORMAT")
                .help("Output format (jpg, png, tiff for images; mp4, mov, avi for video)")
                .default_value("mp4"),
        )
        .arg(
            Arg::new("fps")
                .short('r')
                .long("fps")
                .value_name("RATE")
                .help("Frame rate for video output (frames per second)")
                .default_value("24"),
        )
        .arg(
            Arg::new("quality")
                .short('q')
                .long("quality")
                .value_name("CRF")
                .help("Video quality (CRF: 0-51, lower = better quality, 18-28 recommended). Ignored by prores")
                .default_value("20"),
        )
        .arg(
            Arg::new("codec")
                .long("codec")
                .value_name("CODEC")
                .help("Video codec: h264, h265 or prores (prores requires -f mov)")
                .default_value("h264"),
        )
        .arg(
            Arg::new("ten-bit")
                .long("ten-bit")
                .num_args(0)
                .help("Encode with 10-bit chroma (h265 and prores only)"),
        )
        .arg(
            Arg::new("jpeg-quality")
                .long("jpeg-quality")
                .value_name("QUALITY")
                .help("JPEG quality for image-sequence output (1-100)")
                .default_value("90"),
        )
        .arg(
            Arg::new("progress")
                .long("progress")
                .value_name("MODE")
                .help("Progress output: 'human' (progress bar on stderr) or 'json' (NDJSON events on stdout)")
                .default_value("human"),
        )
        .arg(
            Arg::new("resolution")
                .long("resolution")
                .value_name("WIDTHxHEIGHT")
                .help("Output video resolution (e.g., 1920x1080, 4K, HD). Default: original size"),
        )
        .arg(
            Arg::new("threads")
                .short('t')
                .long("threads")
                .value_name("NUM")
                .help("Number of threads to use for processing (default: auto-detect)")
                .default_value("0"),
        )
        .arg(
            Arg::new("start-frame")
                .long("start-frame")
                .value_name("INDEX")
                .help("Start frame index (0-based, inclusive). Default: 0 (first frame)"),
        )
        .arg(
            Arg::new("end-frame")
                .long("end-frame")
                .value_name("INDEX")
                .help("End frame index (0-based, inclusive). Default: last frame"),
        )
        .arg(
            Arg::new("crop")
                .long("crop")
                .value_name("WIDTH:HEIGHT:X:Y")
                .help("Crop parameters in FFmpeg format (e.g., '1000:800:100:50' or '50%:50%:10%:10%')"),
        )
        .arg(
            Arg::new("offset-x")
                .allow_hyphen_values(true)
                .long("offset-x")
                .value_name("PIXELS")
                .help("X offset for crop window in pixels. Single value or comma-separated array. Examples: '10' (static), '0,20,0,-20' (panning), '0,5,-5,0' (stabilization)"),
        )
        .arg(
            Arg::new("offset-y")
                .allow_hyphen_values(true)
                .long("offset-y")
                .value_name("PIXELS")
                .help("Y offset for crop window in pixels. Single value or comma-separated array. Examples: '-5' (static), '0,10,0,-10' (panning), '0,-3,3,0' (stabilization)"),
        )
}

fn is_explicit(matches: &ArgMatches, name: &str) -> bool {
    matches.value_source(name) == Some(ValueSource::CommandLine)
}

/// Build the project from a project file (if given) with CLI flag overrides,
/// or purely from CLI flags.
fn build_project(matches: &ArgMatches) -> Result<Project> {
    let from_file = matches.get_one::<String>("project").is_some();

    let mut project = match matches.get_one::<String>("project") {
        Some(path) => Project::from_json_file(Path::new(path))?,
        None => {
            let input = matches.get_one::<String>("input").ok_or_else(|| {
                LapsifyError::message(
                    "An input directory is required (-i <DIR> or --project <FILE>)",
                )
            })?;
            // Commands that write into a directory (render) validate the
            // output separately; preview and dump don't need one.
            let output = matches
                .get_one::<String>("output")
                .map(String::as_str)
                .unwrap_or(".");
            Project {
                version: PROJECT_VERSION,
                input: PathBuf::from(input),
                frame_range: None,
                color: ColorGrade::default(),
                crop: None,
                export: ExportSettings::new(PathBuf::from(output)),
            }
        }
    };

    // A flag overrides the project file only when passed explicitly; without
    // a project file, flags (including their defaults) define everything.
    let overrides = |name: &str| !from_file || is_explicit(matches, name);

    if from_file && is_explicit(matches, "input") {
        project.input = PathBuf::from(matches.get_one::<String>("input").unwrap());
    }
    if from_file && is_explicit(matches, "output") {
        project.export.output = PathBuf::from(matches.get_one::<String>("output").unwrap());
    }
    if overrides("format") {
        project.export.format = matches.get_one::<String>("format").unwrap().clone();
    }
    if overrides("fps") {
        project.export.fps = matches
            .get_one::<String>("fps")
            .unwrap()
            .parse::<u32>()
            .map_err(|_| LapsifyError::message("Invalid fps value"))?;
    }
    if overrides("quality") {
        project.export.quality = matches
            .get_one::<String>("quality")
            .unwrap()
            .parse::<u32>()
            .map_err(|_| LapsifyError::message("Invalid quality value"))?;
    }
    if is_explicit(matches, "resolution") {
        project.export.resolution = matches.get_one::<String>("resolution").cloned();
    }
    if overrides("codec") {
        project.export.codec = matches
            .get_one::<String>("codec")
            .unwrap()
            .parse::<Codec>()?;
    }
    if is_explicit(matches, "ten-bit") {
        project.export.ten_bit = true;
    }
    if overrides("jpeg-quality") {
        project.export.jpeg_quality = matches
            .get_one::<String>("jpeg-quality")
            .unwrap()
            .parse::<u8>()
            .map_err(|_| LapsifyError::message("Invalid jpeg-quality value"))?;
    }

    let start_frame = matches
        .get_one::<String>("start-frame")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| LapsifyError::message("Invalid start-frame value"))?;
    let end_frame = matches
        .get_one::<String>("end-frame")
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| LapsifyError::message("Invalid end-frame value"))?;
    if start_frame.is_some() || end_frame.is_some() {
        let image_count = list_images(&project.input)?.len();
        let start = start_frame.unwrap_or(0);
        let end = end_frame.unwrap_or(image_count.saturating_sub(1));
        project.frame_range = Some((start, end));
    }

    // Legacy comma-array flags are anchored to evenly spaced keyframes over
    // the full sequence, so conversion needs the frame count.
    let legacy_curves = ["exposure", "brightness", "contrast", "saturation"]
        .iter()
        .any(|name| overrides(name))
        || is_explicit(matches, "crop")
        || is_explicit(matches, "offset-x")
        || is_explicit(matches, "offset-y");

    if legacy_curves {
        let total_frames = list_images(&project.input)?.len();

        let legacy = |name: &str| -> Result<Curve> {
            let values = parse_value_array(matches.get_one::<String>(name).unwrap())?;
            if values.is_empty() {
                return Err(LapsifyError::message(format!(
                    "Empty value array for {name}"
                )));
            }
            Ok(curve_from_legacy_array(&values, total_frames))
        };

        if overrides("exposure") {
            project.color.exposure = legacy("exposure")?;
        }
        if overrides("brightness") {
            project.color.brightness = legacy("brightness")?;
        }
        if overrides("contrast") {
            project.color.contrast = legacy("contrast")?;
        }
        if overrides("saturation") {
            project.color.saturation = legacy("saturation")?;
        }

        if is_explicit(matches, "crop") {
            let dims = parse_crop_dims(matches.get_one::<String>("crop").unwrap())?;

            let offset = |name: &str| -> Result<Curve> {
                match matches.get_one::<String>(name) {
                    Some(raw) if is_explicit(matches, name) => {
                        let values = parse_value_array(raw)?;
                        Ok(curve_from_legacy_array(&values, total_frames))
                    }
                    _ => Ok(Curve::Constant(0.0)),
                }
            };

            let image_files = list_images(&project.input)?;
            let (src_w, src_h) = scan_dimensions(&image_files)?;
            project.crop = Some(legacy_crop_to_track(
                dims,
                &offset("offset-x")?,
                &offset("offset-y")?,
                src_w,
                src_h,
            )?);
        }
    }

    Ok(project)
}

fn print_curve(name: &str, curve: &Curve, unit: &str) {
    match curve {
        Curve::Constant(v) => eprintln!("  {}: {}{}", name.green(), v, unit),
        Curve::Keyframed(keyframes) => {
            let values_str = keyframes
                .iter()
                .map(|k| format!("{}@{}{}", k.value, k.frame, unit))
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("  {}: [{}]", name.green(), values_str);
        }
    }
}

pub fn run() -> Result<()> {
    let matches = build_command().get_matches();

    match matches.subcommand() {
        Some(("render", sub)) => run_render(sub),
        Some(("preview", sub)) => run_preview(sub),
        Some(("project", sub)) => match sub.subcommand() {
            Some(("dump", dump)) => run_project_dump(dump),
            _ => unreachable!("subcommand_required"),
        },
        Some(_) => unreachable!("unknown subcommand"),
        None => {
            eprintln!(
                "note: running lapsify without a subcommand is deprecated; use `lapsify render`"
            );
            run_render(&matches)
        }
    }
}

fn run_project_dump(matches: &ArgMatches) -> Result<()> {
    let project = build_project(matches)?;
    project.validate()?;
    println!("{}", project.to_json_pretty()?);
    Ok(())
}

fn run_preview(matches: &ArgMatches) -> Result<()> {
    let project = build_project(matches)?;
    project.validate()?;

    let frame = matches
        .get_one::<String>("frame")
        .unwrap()
        .parse::<u32>()
        .map_err(|_| LapsifyError::message("Invalid frame value"))?;
    let max_dim = matches
        .get_one::<String>("max-dim")
        .map(|s| s.parse::<u32>())
        .transpose()
        .map_err(|_| LapsifyError::message("Invalid max-dim value"))?;
    let out = PathBuf::from(matches.get_one::<String>("out").unwrap());

    let preview = crate::render::render_preview(&project, frame, max_dim)?;
    preview.save(&out)?;
    eprintln!("Preview of frame {frame} written to {}", out.display());
    Ok(())
}

fn run_render(matches: &ArgMatches) -> Result<()> {
    let threads = matches
        .get_one::<String>("threads")
        .unwrap()
        .parse::<usize>()
        .map_err(|_| LapsifyError::message("Invalid threads value"))?;

    if threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|e| LapsifyError::message(format!("Failed to configure thread pool: {e}")))?;
    }

    if matches.get_one::<String>("project").is_none()
        && matches.get_one::<String>("output").is_none()
    {
        return Err(LapsifyError::message(
            "An output directory is required (-o <DIR> or --project <FILE>)",
        ));
    }

    let project = build_project(matches)?;
    project.validate()?;

    // Scan every frame's header up front: catches mixed frame sizes before
    // any processing, and validates the crop window against every frame.
    let image_files = list_images(&project.input)?;
    scan_dimensions(&image_files)?;
    if let Some(ref crop) = project.crop {
        crop.validate_over(image_files.len())?;
    }

    let reporter = match matches.get_one::<String>("progress").unwrap().as_str() {
        "human" => ProgressReporter::human(),
        "json" => ProgressReporter::json(),
        other => {
            return Err(LapsifyError::message(format!(
                "Invalid progress mode '{other}' (expected 'human' or 'json')"
            )))
        }
    };

    // All human-readable chatter goes to stderr so json mode keeps stdout
    // clean for NDJSON events.
    if matches!(reporter, ProgressReporter::Human(_)) {
        eprintln!("{}", "Processing images with settings:".bold().cyan());
        print_curve("Exposure", &project.color.exposure, "EV");
        print_curve("Brightness", &project.color.brightness, "");
        print_curve("Contrast", &project.color.contrast, "x");
        print_curve("Saturation", &project.color.saturation, "x");

        if let Some(ref crop) = project.crop {
            eprintln!("  {}:", "Crop".green());
            print_curve("  x", &crop.x, "");
            print_curve("  y", &crop.y, "");
            print_curve("  width", &crop.width, "");
            print_curve("  height", &crop.height, "");
        }

        if project.is_video_output() {
            eprintln!(
                "  {}: {} video at {} fps ({:?}, CRF {})",
                "Output".yellow(),
                project.export.format,
                project.export.fps,
                project.export.codec,
                project.export.quality
            );
            if let Some(ref res) = project.export.resolution {
                eprintln!("  {}: {}", "Resolution".yellow(), res);
            }
        } else {
            eprintln!(
                "  {}: {} images",
                "Output format".yellow(),
                project.export.format
            );
        }

        if let Some((start, end)) = project.frame_range {
            eprintln!(
                "  {}: frames {} to {} ({} frames)",
                "Frame range".yellow(),
                start,
                end,
                end.saturating_sub(start) + 1
            );
        }
    }

    let start_time = Instant::now();

    if project.is_video_output() {
        render_to_video(&project, &reporter, start_time)?;
    } else {
        render_to_images(&project, &reporter, start_time)?;
    }

    Ok(())
}
