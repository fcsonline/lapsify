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
                    .help("Output image file, or '-' for PNG on stdout")
                    .default_value("preview.png"),
            )
            .arg(
                Arg::new("source")
                    .long("source")
                    .num_args(0)
                    .help("Render the ungraded source frame (no color, no crop) — for region picking"),
            ),
    )
    .subcommand(
        Command::new("project")
            .about("Project file utilities")
            .subcommand_required(true)
            .subcommand(render_args(Command::new("dump").about(
                "Print the project JSON that the given flags produce",
            )))
            .subcommand(Command::new("schema").about(
                "Print the JSON Schema of the project file format (for editor validation)",
            )),
    )
    .subcommand(
        render_args(Command::new("deflicker").about(
            "Measure developed luminance, smooth it into a target curve and correct each frame toward it",
        ))
        .arg(
            Arg::new("smoothing")
                .long("smoothing")
                .value_name("FRAMES")
                .help("Low-pass window in frames for the target curve: flicker shorter than this is removed, longer brightness changes are kept")
                .default_value("30"),
        )
        .arg(
            Arg::new("region")
                .long("region")
                .value_name("X,Y,W,H")
                .help("Restrict measurement to a normalized source-image region (0..1 fractions)"),
        )
        .arg(
            Arg::new("passes")
                .long("passes")
                .value_name("N")
                .help("Maximum correction passes")
                .default_value("3"),
        )
        .arg(
            Arg::new("threshold")
                .long("threshold")
                .value_name("EV")
                .help("Per-frame convergence threshold in EV (values below ~0.02 chase 8-bit quantization noise)")
                .default_value("0.03"),
        )
        .arg(
            Arg::new("measure-dim")
                .long("measure-dim")
                .value_name("PIXELS")
                .help("Downscale the long edge to this size before measuring")
                .default_value("256"),
        )
        .arg(
            Arg::new("refine")
                .long("refine")
                .num_args(0)
                .help("Keep the stored target curve and only refine the corrections"),
        )
        .arg(
            Arg::new("reset")
                .long("reset")
                .num_args(0)
                .help("Remove the deflicker layer from the project and exit"),
        ),
    )
    .subcommand(
        Command::new("curves")
            .about("Curve data for external editors")
            .subcommand_required(true)
            .subcommand(render_args(Command::new("dump").about(
                "Print every layer curve sampled per frame as one JSON document",
            ))),
    )
    .subcommand(
        Command::new("keyframes")
            .about("Keyframe utilities")
            .subcommand_required(true)
            .subcommand(
                render_args(Command::new("suggest").about(
                    "Suggest keyframe positions from the luminance progression (denser where brightness changes fast)",
                ))
                .arg(
                    Arg::new("count")
                        .long("count")
                        .value_name("N")
                        .help("Exact number of keyframes to place"),
                )
                .arg(
                    Arg::new("density")
                        .long("density")
                        .value_name("PER_EV")
                        .help("Keyframes per EV of total luminance travel (when --count is not given)")
                        .default_value("1.5"),
                )
                .arg(
                    Arg::new("apply")
                        .long("apply")
                        .num_args(0)
                        .help("Insert the suggested keyframes into the exposure curve at their current values (visually a no-op that gives an editor handles to grab)"),
                ),
            ),
    )
    .subcommand(
        Command::new("analyze")
            .about("Analysis passes that write results back into the project file")
            .subcommand_required(true)
            .subcommand(
                render_args(
                    Command::new("luminance")
                        .about("Measure per-frame mean linear luminance across the sequence"),
                )
                .arg(
                    Arg::new("region")
                        .long("region")
                        .value_name("X,Y,W,H")
                        .help("Restrict measurement to a normalized source-image region (0..1 fractions)"),
                )
                .arg(
                    Arg::new("measure-dim")
                        .long("measure-dim")
                        .value_name("PIXELS")
                        .help("Downscale the long edge to this size before measuring")
                        .default_value("256"),
                )
                .arg(
                    Arg::new("developed")
                        .long("developed")
                        .num_args(0)
                        .help("Measure developed frames (all grading applied) instead of source frames"),
                )
                .arg(
                    Arg::new("no-write")
                        .long("no-write")
                        .num_args(0)
                        .help("Compute and emit events without writing the result into the project file"),
                ),
            )
            .subcommand(
                render_args(Command::new("holygrail").about(
                    "Compute exposure compensation for in-camera exposure changes from EXIF",
                ))
                .arg(
                    Arg::new("rotate")
                        .allow_hyphen_values(true)
                        .long("rotate")
                        .value_name("EV")
                        .help("Linear baseline tilt over the clip, in EV. Default: auto-fit so the compensation ends at 0"),
                )
                .arg(
                    Arg::new("stretch")
                        .allow_hyphen_values(true)
                        .long("stretch")
                        .value_name("FACTOR")
                        .help("Scale of the whole compensation")
                        .default_value("1.0"),
                )
                .arg(
                    Arg::new("no-write")
                        .long("no-write")
                        .num_args(0)
                        .help("Compute and emit events without writing the result into the project file"),
                ),
            ),
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
            Arg::new("temperature")
                .allow_hyphen_values(true)
                .long("temperature")
                .value_name("VALUE")
                .help("White balance temperature, -100 (cool) to +100 (warm). Single value or comma-separated array")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("tint")
                .allow_hyphen_values(true)
                .long("tint")
                .value_name("VALUE")
                .help("White balance tint, -100 (green) to +100 (magenta). Single value or comma-separated array")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("highlights")
                .allow_hyphen_values(true)
                .long("highlights")
                .value_name("VALUE")
                .help("Highlight recovery/boost, -100 to +100. Single value or comma-separated array")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("shadows")
                .allow_hyphen_values(true)
                .long("shadows")
                .value_name("VALUE")
                .help("Shadow lift/crush, -100 to +100. Single value or comma-separated array")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("whites")
                .allow_hyphen_values(true)
                .long("whites")
                .value_name("VALUE")
                .help("White point adjustment, -100 to +100. Single value or comma-separated array")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("blacks")
                .allow_hyphen_values(true)
                .long("blacks")
                .value_name("VALUE")
                .help("Black point adjustment, -100 to +100. Single value or comma-separated array")
                .default_value("0.0"),
        )
        .arg(
            Arg::new("gamma")
                .long("gamma")
                .value_name("VALUE")
                .help("Midtone gamma, 0.2 to 5.0 (1.0 = neutral). Single value or comma-separated array")
                .default_value("1.0"),
        )
        .arg(
            Arg::new("vibrance")
                .allow_hyphen_values(true)
                .long("vibrance")
                .value_name("VALUE")
                .help("Vibrance, -100 to +100: saturation boost weighted toward muted colors. Single value or comma-separated array")
                .default_value("0.0"),
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
            Arg::new("motion-blur")
                .long("motion-blur")
                .value_name("FRAMES")
                .help("Motion blur as frame blending: average this many neighboring frames per output frame (2-128, video only)"),
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
            Arg::new("no-holy-grail")
                .long("no-holy-grail")
                .num_args(0)
                .help("Ignore the holy-grail exposure compensation layer for this run (A/B comparison)"),
        )
        .arg(
            Arg::new("no-deflicker")
                .long("no-deflicker")
                .num_args(0)
                .help("Ignore the deflicker correction layer for this run (A/B comparison)"),
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
    let mut project = build_project_inner(matches)?;
    if matches.get_flag("no-holy-grail") {
        if let Some(ref mut analysis) = project.analysis {
            analysis.holy_grail = None;
        }
    }
    if matches.get_flag("no-deflicker") {
        if let Some(ref mut analysis) = project.analysis {
            analysis.deflicker = None;
        }
    }
    Ok(project)
}

fn build_project_inner(matches: &ArgMatches) -> Result<Project> {
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
                interpolation: Default::default(),
                color: ColorGrade::default(),
                crop: None,
                export: ExportSettings::new(PathBuf::from(output)),
                analysis: None,
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
    if is_explicit(matches, "motion-blur") {
        project.export.motion_blur = Some(
            matches
                .get_one::<String>("motion-blur")
                .unwrap()
                .parse::<u32>()
                .map_err(|_| LapsifyError::message("Invalid motion-blur value"))?,
        );
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
    let legacy_curves = [
        "exposure",
        "temperature",
        "tint",
        "brightness",
        "contrast",
        "highlights",
        "shadows",
        "whites",
        "blacks",
        "gamma",
        "saturation",
        "vibrance",
    ]
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

        let color = &mut project.color;
        let fields: [(&str, &mut Curve); 12] = [
            ("exposure", &mut color.exposure),
            ("temperature", &mut color.temperature),
            ("tint", &mut color.tint),
            ("brightness", &mut color.brightness),
            ("contrast", &mut color.contrast),
            ("highlights", &mut color.highlights),
            ("shadows", &mut color.shadows),
            ("whites", &mut color.whites),
            ("blacks", &mut color.blacks),
            ("gamma", &mut color.gamma),
            ("saturation", &mut color.saturation),
            ("vibrance", &mut color.vibrance),
        ];
        for (name, field) in fields {
            if !from_file || is_explicit(matches, name) {
                *field = legacy(name)?;
            }
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
            Some(("schema", _)) => {
                let schema = schemars::schema_for!(Project);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&schema).map_err(|e| {
                        LapsifyError::message(format!("Failed to serialize schema: {e}"))
                    })?
                );
                Ok(())
            }
            _ => unreachable!("subcommand_required"),
        },
        Some(("deflicker", sub)) => run_deflicker_cmd(sub),
        Some(("keyframes", sub)) => match sub.subcommand() {
            Some(("suggest", suggest)) => run_keyframes_suggest(suggest),
            _ => unreachable!("subcommand_required"),
        },
        Some(("curves", sub)) => match sub.subcommand() {
            Some(("dump", dump)) => run_curves_dump(dump),
            _ => unreachable!("subcommand_required"),
        },
        Some(("analyze", sub)) => match sub.subcommand() {
            Some(("luminance", lum)) => run_analyze_luminance(lum),
            Some(("holygrail", hg)) => run_analyze_holygrail(hg),
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

fn run_analyze_luminance(matches: &ArgMatches) -> Result<()> {
    use crate::analysis::luminance::{measure_luminance, parse_region, LuminanceOptions};
    use crate::analysis::Analysis;
    use crate::progress::ProgressEvent;

    let mut project = build_project(matches)?;
    project.validate()?;

    let opts = LuminanceOptions {
        region: matches
            .get_one::<String>("region")
            .map(|s| parse_region(s))
            .transpose()?,
        measure_dim: matches
            .get_one::<String>("measure-dim")
            .unwrap()
            .parse::<u32>()
            .map_err(|_| LapsifyError::message("Invalid measure-dim value"))?,
        developed: matches.get_flag("developed"),
    };

    let reporter = match matches.get_one::<String>("progress").unwrap().as_str() {
        "json" => ProgressReporter::json(),
        _ => ProgressReporter::human(),
    };

    let image_files = list_images(&project.input)?;
    let (width, height) = scan_dimensions(&image_files)?;
    reporter.report(ProgressEvent::Start {
        total_frames: image_files.len(),
        width,
        height,
    });

    let start = Instant::now();
    let series = measure_luminance(&project, &image_files, &opts, &reporter)?;

    let project_path = matches.get_one::<String>("project").map(PathBuf::from);
    let written = match (&project_path, matches.get_flag("no-write")) {
        (Some(path), false) => {
            let analysis = project.analysis.get_or_insert_with(Analysis::default);
            if opts.developed {
                analysis.developed_luminance = Some(series);
            } else {
                analysis.source_luminance = Some(series);
            }
            project.save_atomic(path)?;
            path.clone()
        }
        _ => {
            if project_path.is_none() {
                reporter.report(ProgressEvent::Warning {
                    message: "no project file given; results were not persisted (use --project)"
                        .to_string(),
                });
            }
            PathBuf::new()
        }
    };

    reporter.report(ProgressEvent::Done {
        output: written,
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Everything a curve editor needs to draw the layer graph, in one call:
/// measured luminance series, the machine layers, and the user exposure
/// curve sampled per frame, plus their sum.
fn run_curves_dump(matches: &ArgMatches) -> Result<()> {
    use crate::timeline::Timeline;

    let project = build_project(matches)?;
    project.validate()?;

    let n = list_images(&project.input)?.len();
    let timeline = Timeline::of(&project);
    let analysis = project.analysis.as_ref();

    let user_exposure: Vec<f32> = (0..n as u32)
        .map(|f| project.color.exposure.sample_mapped(f, |x| timeline.x(x)))
        .collect();
    let holy_grail: Option<Vec<f32>> = analysis
        .and_then(|a| a.holy_grail.as_ref())
        .map(|hg| (0..n).map(|f| hg.effective(f)).collect());
    let deflicker: Option<Vec<f32>> = analysis
        .and_then(|a| a.deflicker.as_ref())
        .map(|d| (0..n).map(|f| d.offset(f)).collect());
    let effective: Vec<f32> = (0..n)
        .map(|f| {
            user_exposure[f]
                + holy_grail.as_ref().map_or(0.0, |v| v[f])
                + deflicker.as_ref().map_or(0.0, |v| v[f])
        })
        .collect();

    let document = serde_json::json!({
        "frames": n,
        "capture_times_ms": analysis.and_then(|a| a.capture_times_ms.clone()),
        "layers": {
            "source_luminance": analysis.and_then(|a| a.source_luminance.as_ref()).map(|s| &s.values),
            "developed_luminance": analysis.and_then(|a| a.developed_luminance.as_ref()).map(|s| &s.values),
            "deflicker_target": analysis.and_then(|a| a.deflicker.as_ref()).map(|d| &d.target),
            "holy_grail_ev": holy_grail,
            "deflicker_ev": deflicker,
            "user_exposure_ev": user_exposure,
            "effective_exposure_ev": effective,
        },
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&document)
            .map_err(|e| LapsifyError::message(format!("Failed to serialize curves: {e}")))?
    );
    Ok(())
}

fn run_keyframes_suggest(matches: &ArgMatches) -> Result<()> {
    use crate::analysis::keyframes::{suggest_keyframes, SuggestOptions};
    use crate::curve::Keyframe;
    use crate::progress::ProgressEvent;

    let mut project = build_project(matches)?;
    project.validate()?;

    let source_luma = project
        .analysis
        .as_ref()
        .and_then(|a| a.source_luminance.as_ref())
        .map(|s| s.values.clone())
        .ok_or_else(|| {
            LapsifyError::message(
                "Keyframe suggestion needs source luminance; run `lapsify analyze luminance --project <FILE>` first",
            )
        })?;
    let holy_grail = project.analysis.as_ref().and_then(|a| a.holy_grail.clone());

    let opts = SuggestOptions {
        count: matches
            .get_one::<String>("count")
            .map(|s| s.parse::<usize>())
            .transpose()
            .map_err(|_| LapsifyError::message("Invalid count value"))?,
        density: matches
            .get_one::<String>("density")
            .unwrap()
            .parse::<f32>()
            .map_err(|_| LapsifyError::message("Invalid density value"))?,
    };

    let reporter = match matches.get_one::<String>("progress").unwrap().as_str() {
        "json" => ProgressReporter::json(),
        _ => ProgressReporter::human(),
    };

    let frames = suggest_keyframes(&source_luma, holy_grail.as_ref(), &opts)?;
    reporter.report(ProgressEvent::KeyframeSuggestion {
        frames: frames.clone(),
    });

    if matches.get_flag("apply") {
        let project_path = matches.get_one::<String>("project").ok_or_else(|| {
            LapsifyError::message("--apply writes into the project file; pass --project <FILE>")
        })?;

        // Insert keyframes at the curve's current sampled values: the render
        // is unchanged, but an editor now has handles at the right places.
        // Existing keyframes within 2 frames of a suggestion are kept as-is.
        let existing: Vec<u32> = match &project.color.exposure {
            Curve::Keyframed(kfs) => kfs.iter().map(|k| k.frame).collect(),
            Curve::Constant(_) => Vec::new(),
        };
        let mut keyframes: Vec<Keyframe> = match &project.color.exposure {
            Curve::Keyframed(kfs) => kfs.clone(),
            Curve::Constant(_) => Vec::new(),
        };
        for &frame in &frames {
            if existing.iter().any(|&e| e.abs_diff(frame) <= 2) {
                continue;
            }
            keyframes.push(Keyframe::new(frame, project.color.exposure.sample(frame)));
        }
        keyframes.sort_by_key(|k| k.frame);
        project.color.exposure = Curve::Keyframed(keyframes);

        project.save_atomic(Path::new(project_path))?;
        reporter.report(ProgressEvent::Done {
            output: PathBuf::from(project_path),
            elapsed_ms: 0,
        });
    }

    Ok(())
}

fn run_deflicker_cmd(matches: &ArgMatches) -> Result<()> {
    use crate::analysis::deflicker::{run_deflicker, DeflickerOptions};
    use crate::analysis::luminance::parse_region;
    use crate::analysis::Analysis;
    use crate::progress::ProgressEvent;

    let project_path = matches
        .get_one::<String>("project")
        .map(PathBuf::from)
        .ok_or_else(|| {
            LapsifyError::message(
                "deflicker stores its corrections in the project file; pass --project <FILE>",
            )
        })?;

    // Load the file directly (not via flag overrides) so what is written
    // back is exactly the stored project plus the deflicker layer.
    let mut project = Project::from_json_file(&project_path)?;
    project.validate()?;

    let reporter = match matches.get_one::<String>("progress").unwrap().as_str() {
        "json" => ProgressReporter::json(),
        _ => ProgressReporter::human(),
    };

    if matches.get_flag("reset") {
        if let Some(ref mut analysis) = project.analysis {
            analysis.deflicker = None;
        }
        project.save_atomic(&project_path)?;
        reporter.report(ProgressEvent::Done {
            output: project_path,
            elapsed_ms: 0,
        });
        return Ok(());
    }

    let opts = DeflickerOptions {
        smoothing_frames: matches
            .get_one::<String>("smoothing")
            .unwrap()
            .parse::<u32>()
            .map_err(|_| LapsifyError::message("Invalid smoothing value"))?,
        region: matches
            .get_one::<String>("region")
            .map(|s| parse_region(s))
            .transpose()?,
        max_passes: matches
            .get_one::<String>("passes")
            .unwrap()
            .parse::<u32>()
            .map_err(|_| LapsifyError::message("Invalid passes value"))?,
        threshold_ev: matches
            .get_one::<String>("threshold")
            .unwrap()
            .parse::<f32>()
            .map_err(|_| LapsifyError::message("Invalid threshold value"))?,
        measure_dim: matches
            .get_one::<String>("measure-dim")
            .unwrap()
            .parse::<u32>()
            .map_err(|_| LapsifyError::message("Invalid measure-dim value"))?,
        refine: matches.get_flag("refine"),
    };

    let image_files = list_images(&project.input)?;
    let (width, height) = scan_dimensions(&image_files)?;
    reporter.report(ProgressEvent::Start {
        total_frames: image_files.len(),
        width,
        height,
    });

    let start = Instant::now();
    let layer = run_deflicker(&project, &image_files, &opts, &reporter)?;

    if !layer.converged {
        reporter.report(ProgressEvent::Warning {
            message: format!(
                "did not fully converge after {} pass(es); re-run or raise --passes",
                layer.passes_run
            ),
        });
    }

    let analysis = project.analysis.get_or_insert_with(Analysis::default);
    analysis.deflicker = Some(layer);
    project.save_atomic(&project_path)?;

    reporter.report(ProgressEvent::Done {
        output: project_path,
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

fn run_analyze_holygrail(matches: &ArgMatches) -> Result<()> {
    use crate::analysis::holygrail::{compute_holy_grail, HolyGrailOptions};
    use crate::analysis::Analysis;
    use crate::progress::ProgressEvent;

    let mut project = build_project(matches)?;
    project.validate()?;

    let opts = HolyGrailOptions {
        rotate: matches
            .get_one::<String>("rotate")
            .map(|s| s.parse::<f32>())
            .transpose()
            .map_err(|_| LapsifyError::message("Invalid rotate value"))?,
        stretch: matches
            .get_one::<String>("stretch")
            .map(|s| s.parse::<f32>())
            .transpose()
            .map_err(|_| LapsifyError::message("Invalid stretch value"))?,
    };

    let reporter = match matches.get_one::<String>("progress").unwrap().as_str() {
        "json" => ProgressReporter::json(),
        _ => ProgressReporter::human(),
    };

    let image_files = list_images(&project.input)?;
    reporter.report(ProgressEvent::Start {
        total_frames: image_files.len(),
        width: 0,
        height: 0,
    });

    let start = Instant::now();
    let (layer, capture_times) = compute_holy_grail(&image_files, &opts, &reporter)?;

    if !layer.frames_missing_exif.is_empty() {
        reporter.report(ProgressEvent::Warning {
            message: format!(
                "{} frame(s) had no usable EXIF exposure data; their compensation was carried forward",
                layer.frames_missing_exif.len()
            ),
        });
    }

    let project_path = matches.get_one::<String>("project").map(PathBuf::from);
    let written = match (&project_path, matches.get_flag("no-write")) {
        (Some(path), false) => {
            let analysis = project.analysis.get_or_insert_with(Analysis::default);
            analysis.holy_grail = Some(layer);
            if capture_times.is_some() {
                analysis.capture_times_ms = capture_times;
            }
            project.save_atomic(path)?;
            path.clone()
        }
        _ => {
            if project_path.is_none() {
                reporter.report(ProgressEvent::Warning {
                    message: "no project file given; results were not persisted (use --project)"
                        .to_string(),
                });
            }
            PathBuf::new()
        }
    };

    reporter.report(ProgressEvent::Done {
        output: written,
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
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
    let out = matches.get_one::<String>("out").unwrap();

    let preview = if matches.get_flag("source") {
        // Ungraded, uncropped source frame (downscaled), aligned with the
        // coordinates that regions and the crop track are defined in.
        let files = list_images(&project.input)?;
        let path = files.get(frame as usize).ok_or_else(|| {
            LapsifyError::message(format!(
                "Frame {frame} is out of range (0-{})",
                files.len().saturating_sub(1)
            ))
        })?;
        let mut img = crate::source::load_frame(path)?;
        if let Some(dim) = max_dim {
            if img.width() > dim || img.height() > dim {
                img = img.thumbnail(dim, dim);
            }
        }
        img
    } else {
        crate::render::render_preview(&project, frame, max_dim)?
    };

    if out == "-" {
        let mut bytes = Vec::new();
        preview.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )?;
        use std::io::Write;
        std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| LapsifyError::message(format!("Failed to write PNG to stdout: {e}")))?;
    } else {
        let out = PathBuf::from(out);
        preview.save(&out)?;
        eprintln!("Preview of frame {frame} written to {}", out.display());
    }
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
