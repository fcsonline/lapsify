
![Lapsify Logo](logo.png)

# Lapsify

A time-lapse processing engine written in Rust: keyframable adjustments over an
image sequence, rendered to video (via FFmpeg) or processed stills. Usable as a
CLI or as a Rust library.

## Features

- **Keyframed adjustments**: anchor exposure, brightness, contrast and
  saturation values to specific frames; smooth monotone interpolation between
  keyframes (the curve passes through every keyframe, with no overshoot)
- **Correct color math**: exposure is applied in linear light (real EV stops);
  the whole tonal chain is baked into a per-frame LUT for speed
- **Keyframable crop**: a normalized crop window whose position and size can be
  animated — pans and Ken Burns zooms are just keyframes
- **Project files**: describe a whole render in JSON; the CLI flags and the
  project file build the exact same pipeline
- **Direct video encoding**: frames are piped straight into FFmpeg (no
  temporary files, no intermediate compression); H.264, H.265 (8/10-bit) and
  ProRes
- **Machine-readable progress**: `--progress json` emits NDJSON events on
  stdout, designed for driving lapsify from another program or UI
- **Parallel processing**: frames render across all cores with bounded memory

## Installation

### Prerequisites

- Rust (1.70 or later)
- FFmpeg (for video output)

### Installing with Cargo

```bash
cargo install lapsify
```

### Building from Source

```bash
git clone https://github.com/fcsonline/lapsify.git
cd lapsify
cargo build --release
```

## Usage

```bash
# Process images to video
lapsify -i /path/to/images -o /path/to/output -f mp4

# Process images to processed images
lapsify -i /path/to/images -o /path/to/output -f jpg

# Render from a project file
lapsify --project project.json
```

### Command Line Options

- `-p, --project <FILE>`: JSON project file; other flags override its values
- `-i, --input <DIR>`: Input directory containing images
- `-o, --output <DIR>`: Output directory for processed files
- `-e, --exposure <STOPS>`: Exposure in EV stops (-3.0 to +3.0)
- `-b, --brightness <VALUE>`: Brightness adjustment (-100 to +100)
- `-c, --contrast <VALUE>`: Contrast multiplier (0.1 to 3.0)
- `-s, --saturation <VALUE>`: Saturation multiplier (0.0 to 2.0)
- `--crop <WIDTH:HEIGHT:X:Y>`: Crop window (pixels, or percentages with `%`)
- `--offset-x <PIXELS>`, `--offset-y <PIXELS>`: Crop window offsets over time
- `-f, --format <FORMAT>`: jpg, png, tiff (images) or mp4, mov, avi (video)
- `-r, --fps <RATE>`: Video frame rate (1-120)
- `-q, --quality <CRF>`: Video quality (0-51, lower = better)
- `--codec <CODEC>`: h264 (default), h265 or prores (prores requires `-f mov`)
- `--ten-bit`: 10-bit chroma (h265/prores)
- `--jpeg-quality <1-100>`: JPEG quality for image output (default 90)
- `--resolution <WIDTHxHEIGHT>`: Fit output within this size (e.g. 1920x1080, 4K)
- `--start-frame <N>`, `--end-frame <N>`: Inclusive frame range (0-based)
- `--progress <human|json>`: Progress bar on stderr, or NDJSON events on stdout
- `-t, --threads <NUM>`: Worker threads (default: all cores)

### Value arrays

Adjustment flags accept a single value or a comma-separated array. Array values
become keyframes spread evenly across the clip, and the rendered curve passes
through every value:

```bash
# Ramp exposure from -1 EV to +1 EV
lapsify -i images/ -o out/ -e "-1.0,1.0" -f mp4

# Brightness dips mid-clip
lapsify -i images/ -o out/ -b "0,20,-10,0" -f mp4
```

### Project files

A project file is the full description of a render — the same model the CLI
flags build internally, with per-frame keyframe control:

```json
{
  "version": 1,
  "input": "frames/",
  "color": {
    "exposure": [
      { "frame": 0, "value": -0.5 },
      { "frame": 120, "value": 1.0, "easing": "linear" },
      { "frame": 300, "value": 0.0 }
    ],
    "contrast": 1.1
  },
  "crop": {
    "x": [ { "frame": 0, "value": 0.0 }, { "frame": 300, "value": 0.25 } ],
    "y": 0.0,
    "width": [ { "frame": 0, "value": 1.0 }, { "frame": 300, "value": 0.5 } ],
    "height": [ { "frame": 0, "value": 1.0 }, { "frame": 300, "value": 0.5 } ]
  },
  "export": {
    "output": "out/",
    "format": "mp4",
    "fps": 24,
    "quality": 18,
    "codec": "h265",
    "resolution": "4K"
  }
}
```

- Every adjustment is either a constant (`"contrast": 1.1`) or a keyframe list.
- Keyframe easings: `smooth` (default), `linear`, `hold`, `ease_in`,
  `ease_out`, `ease_in_out`.
- The crop window uses normalized coordinates (0..1 fractions of the source
  image), so a project is resolution-independent. Animating `width`/`height`
  zooms; animating `x`/`y` pans.
- Flags passed alongside `--project` override the file's values.

### Cropping (flags)

```bash
# Pixel window: 600x400 at (100, 50)
lapsify -i images/ -o out/ --crop "600:400:100:50" -f mp4

# Percentages need the % sign
lapsify -i images/ -o out/ --crop "50%:50%:25%:25%" -f mp4

# Negative pixel values anchor to the right/bottom edge
lapsify -i images/ -o out/ --crop "600:400:-100:-50" -f mp4

# Animated offsets: pan the window while rendering
lapsify -i images/ -o out/ --crop "3000:2400:0:0" --offset-x "0,150" -f mp4
```

Bare numbers are always pixels; use `%` for percentages. The crop window is
validated against every frame before processing starts.

### Driving lapsify from another program

```bash
lapsify -i frames/ -o out/ -f mp4 --progress json
```

```
{"event":"start","total_frames":5,"width":320,"height":240}
{"event":"frame","index":0,"done":1,"total":5}
...
{"event":"done","output":"out/timelapse.mp4","elapsed_ms":156}
```

Stdout carries only NDJSON events; all human-readable output goes to stderr.

## Library usage

The `lapsify` crate exposes the engine directly:

```rust
use lapsify::{Project, render_frame};

let project = Project::from_json_file("project.json".as_ref())?;
let frame = render_frame(image::open("frames/0001.jpg")?, &project, 42)?;
```

## Supported Image Formats

- JPEG (.jpg, .jpeg)
- PNG (.png)
- TIFF (.tiff, .tif)
- BMP (.bmp)
- WebP (.webp)
- ⚠️ Pending: RAW formats (.cr2, .nef, .arw)

## License

MIT License - see LICENSE file for details.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request
