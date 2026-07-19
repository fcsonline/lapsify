use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use image::{ImageBuffer, Rgb};
use predicates::prelude::*;

fn write_frames(dir: &Path, count: u32) {
    fs::create_dir_all(dir).unwrap();
    for i in 0..count {
        let img = ImageBuffer::from_fn(64, 48, |x, y| {
            Rgb([
                ((x + i * 10) * 3 % 256) as u8,
                ((y + i * 5) * 4 % 256) as u8,
                ((x + y) * 2 % 256) as u8,
            ])
        });
        img.save(dir.join(format!("frame_{i:03}.png"))).unwrap();
    }
}

fn lapsify() -> Command {
    Command::cargo_bin("lapsify").unwrap()
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn renders_image_sequence_from_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    let output = tmp.path().join("out");
    write_frames(&input, 5);

    lapsify()
        .args(["-i", input.to_str().unwrap()])
        .args(["-o", output.to_str().unwrap()])
        .args(["-f", "jpg", "-e", "0.5"])
        .assert()
        .success();

    let outputs: Vec<_> = fs::read_dir(&output)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().into_string().unwrap())
        .collect();
    assert_eq!(outputs.len(), 5);
    assert!(outputs.iter().all(|name| name.ends_with("_processed.jpg")));
}

#[test]
fn project_file_and_flags_produce_identical_output() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    write_frames(&input, 4);

    let out_flags = tmp.path().join("out_flags");
    lapsify()
        .args(["-i", input.to_str().unwrap()])
        .args(["-o", out_flags.to_str().unwrap()])
        .args(["-f", "png", "-e", "-0.5,0.5", "-c", "1.2"])
        .assert()
        .success();

    // The equivalent project: legacy arrays spread evenly over 4 frames.
    let out_project = tmp.path().join("out_project");
    let project = serde_json::json!({
        "version": 1,
        "input": input.to_str().unwrap(),
        "color": {
            "exposure": [
                {"frame": 0, "value": -0.5},
                {"frame": 3, "value": 0.5}
            ],
            "contrast": 1.2
        },
        "export": { "output": out_project.to_str().unwrap(), "format": "png" }
    });
    let project_path = tmp.path().join("project.json");
    fs::write(
        &project_path,
        serde_json::to_string_pretty(&project).unwrap(),
    )
    .unwrap();

    lapsify()
        .args(["--project", project_path.to_str().unwrap()])
        .assert()
        .success();

    for i in 0..4 {
        let name = format!("frame_{i:03}_processed.png");
        let a = fs::read(out_flags.join(&name)).unwrap();
        let b = fs::read(out_project.join(&name)).unwrap();
        assert_eq!(a, b, "frame {i} differs between flags and project file");
    }
}

#[test]
fn json_progress_emits_parsable_ndjson() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    let output = tmp.path().join("out");
    write_frames(&input, 3);

    let assert = lapsify()
        .args(["-i", input.to_str().unwrap()])
        .args(["-o", output.to_str().unwrap()])
        .args(["-f", "jpg", "--progress", "json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every stdout line is JSON"))
        .collect();

    assert!(!events.is_empty());
    assert_eq!(events[0]["event"], "start");
    assert_eq!(events[0]["total_frames"], 3);
    assert!(events.iter().any(|e| e["event"] == "frame"));
    assert_eq!(events.last().unwrap()["event"], "done");
}

#[test]
fn rejects_mixed_frame_sizes_before_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    write_frames(&input, 2);
    // One frame with a different size.
    ImageBuffer::from_pixel(32, 32, Rgb([1u8, 2, 3]))
        .save(input.join("frame_999.png"))
        .unwrap();

    lapsify()
        .args(["-i", input.to_str().unwrap()])
        .args(["-o", tmp.path().join("out").to_str().unwrap()])
        .args(["-f", "jpg"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("expected"));
}

#[test]
fn encodes_mp4_via_ffmpeg_pipe() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    let output = tmp.path().join("out");
    write_frames(&input, 5);

    lapsify()
        .args(["-i", input.to_str().unwrap()])
        .args(["-o", output.to_str().unwrap()])
        .args(["-f", "mp4", "-r", "5", "-e", "0.3"])
        .assert()
        .success();

    let video = output.join("timelapse.mp4");
    assert!(video.exists());
    assert!(fs::metadata(&video).unwrap().len() > 0);

    // No temp frame directory left behind.
    assert!(!output.join("temp_frames").exists());

    // If ffprobe is around, verify the frame count survived the pipe.
    if let Ok(probe) = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "csv=p=0",
        ])
        .arg(&video)
        .output()
    {
        if probe.status.success() {
            let count = String::from_utf8_lossy(&probe.stdout).trim().to_string();
            assert_eq!(count, "5");
        }
    }
}

#[test]
fn render_subcommand_matches_legacy_invocation() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    write_frames(&input, 3);

    let out_legacy = tmp.path().join("out_legacy");
    lapsify()
        .args(["-i", input.to_str().unwrap()])
        .args(["-o", out_legacy.to_str().unwrap()])
        .args(["-f", "png", "-e", "0.5"])
        .assert()
        .success()
        .stderr(predicate::str::contains("deprecated"));

    let out_sub = tmp.path().join("out_sub");
    lapsify()
        .arg("render")
        .args(["-i", input.to_str().unwrap()])
        .args(["-o", out_sub.to_str().unwrap()])
        .args(["-f", "png", "-e", "0.5"])
        .assert()
        .success()
        .stderr(predicate::str::contains("deprecated").not());

    for i in 0..3 {
        let name = format!("frame_{i:03}_processed.png");
        assert_eq!(
            fs::read(out_legacy.join(&name)).unwrap(),
            fs::read(out_sub.join(&name)).unwrap()
        );
    }
}

#[test]
fn preview_renders_one_downscaled_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    write_frames(&input, 3);
    let out = tmp.path().join("preview.png");

    lapsify()
        .arg("preview")
        .args(["-i", input.to_str().unwrap()])
        .args(["--frame", "1", "--max-dim", "32"])
        .args(["--out", out.to_str().unwrap()])
        .args(["-e", "1.0"])
        .assert()
        .success();

    let img = image::open(&out).unwrap();
    assert!(img.width() <= 32 && img.height() <= 32);

    // Out-of-range frame errors cleanly.
    lapsify()
        .arg("preview")
        .args(["-i", input.to_str().unwrap()])
        .args(["--frame", "99"])
        .args(["--out", tmp.path().join("x.png").to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("out of range"));
}

#[test]
fn project_dump_prints_valid_project_json() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    write_frames(&input, 4);

    let assert = lapsify()
        .args(["project", "dump"])
        .args(["-i", input.to_str().unwrap()])
        .args(["-e", "-0.5,0.5", "-f", "png"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["version"], 1);
    // The legacy array became keyframes anchored to the sequence.
    assert_eq!(value["color"]["exposure"][0]["frame"], 0);
    assert_eq!(value["color"]["exposure"][1]["frame"], 3);
}

#[test]
fn keyframed_crop_video_encodes_fixed_size() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not available");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("frames");
    let output = tmp.path().join("out");
    write_frames(&input, 5);

    let project = serde_json::json!({
        "version": 1,
        "input": input.to_str().unwrap(),
        "crop": {
            // Zooming crop: the window shrinks over time (Ken Burns).
            "x": [{"frame": 0, "value": 0.0}, {"frame": 4, "value": 0.25}],
            "y": 0.0,
            "width": [{"frame": 0, "value": 1.0}, {"frame": 4, "value": 0.5}],
            "height": [{"frame": 0, "value": 1.0}, {"frame": 4, "value": 0.5}]
        },
        "export": {
            "output": output.to_str().unwrap(),
            "format": "mp4",
            "fps": 5
        }
    });
    let project_path = tmp.path().join("project.json");
    fs::write(&project_path, project.to_string()).unwrap();

    lapsify()
        .args(["--project", project_path.to_str().unwrap()])
        .assert()
        .success();

    assert!(output.join("timelapse.mp4").exists());
}
