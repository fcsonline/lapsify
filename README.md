# Lapsify

A powerful time-lapse image processor written in Rust that can process images with adjustable parameters and create videos.

## Features

- **Image Processing**: Apply exposure, brightness, contrast, and saturation adjustments
- **Cropping**: Crop images with pixel or percentage-based coordinates
- **Interpolation**: Smooth transitions between parameter values across frames
- **Multiple Output Formats**: Generate processed images (JPG, PNG, TIFF) or videos (MP4, MOV, AVI)
- **Video Creation**: Direct video output using FFmpeg with customizable quality and frame rate
- **Flexible Parameters**: Support for single values or arrays for smooth transitions

## Installation

### Prerequisites

- Rust (1.70 or later)
- FFmpeg (for video output)

### Building from Source

```bash
git clone https://github.com/yourusername/lapsify.git
cd lapsify
cargo build --release
```

## Usage

### Basic Usage

```bash
# Process images to video
cargo run --release -- -i /path/to/images -o /path/to/output -f mp4

# Process images to processed images
cargo run --release -- -i /path/to/images -o /path/to/output -f jpg
```

### Command Line Options

- `-i, --input <DIR>`: Input directory containing images (required)
- `-o, --output <DIR>`: Output directory for processed files (required)
- `-e, --exposure <STOPS>`: Exposure adjustment in EV stops (-3.0 to +3.0)
- `-b, --brightness <VALUE>`: Brightness adjustment (-100 to +100)
- `-c, --contrast <VALUE>`: Contrast multiplier (0.1 to 3.0)
- `-s, --saturation <VALUE>`: Saturation multiplier (0.0 to 2.0)
- `--crop <WIDTH:HEIGHT:X:Y>`: Crop parameters in FFmpeg format (e.g., '1000:800:100:50' or '50%:50%:10%:10%')
- `-f, --format <FORMAT>`: Output format (jpg, png, tiff, mp4, mov, avi)
- `-r, --fps <RATE>`: Frame rate for video output (1-120 fps)
- `-q, --quality <CRF>`: Video quality (0-51, lower = better)
- `--resolution <WIDTHxHEIGHT>`: Output video resolution
- `-t, --threads <NUM>`: Number of threads to use for processing (default: auto-detect)

### Cropping

Crop images using FFmpeg-style crop parameters:

```bash
# Crop with pixel coordinates (width:height:x:y)
cargo run --release -- -i images/ -o output/ --crop="600:400:100:50" -f mp4

# Crop with percentage coordinates
cargo run --release -- -i images/ -o output/ --crop="50%:50%:10%:10%" -f mp4

# Crop from right/bottom using negative offsets
cargo run --release -- -i images/ -o output/ --crop="600:400:-100:-100" -f mp4
```

**Crop Format:** `width:height:x:y`
- **Width/Height**: Output dimensions in pixels or percentages
- **X/Y**: Offset coordinates in pixels or percentages
- **Percentages**: Values like '50%' are calculated relative to image dimensions
- **Negative Offsets**: Useful for cropping from the right or bottom edges

### Parameter Arrays

You can provide arrays of values for smooth transitions:

```bash
# Gradual exposure change from -1 to +1 EV
cargo run --release -- -i images/ -o output/ -e "-1.0,1.0" -f mp4

# Multiple brightness points
cargo run --release -- -i images/ -o output/ -b "0,20,-10,0" -f mp4

# Complex contrast curve
cargo run --release -- -i images/ -o output/ -c "1.0,1.5,0.8,1.2" -f mp4
```

### Important Note: Negative Values

When using negative values in command-line arguments, you must use the `=` syntax to separate the argument name from the value:

```bash
# ✅ Correct - use equals sign for negative values
cargo run --release -- --exposure="-1,0.2" --fps 20

# ❌ Incorrect - will be interpreted as separate flags
cargo run --release -- --exposure "-1,0.2" --fps 20
```

This is because the command-line parser interprets values starting with `-` as separate flags unless explicitly bound with `=`.

### Examples

```bash
# Create a video with increased brightness and contrast
cargo run --release -- -i photos/ -o video/ -b 20 -c 1.2 -f mp4 -r 30

# Process with exposure ramping
cargo run --release -- -i photos/ -o processed/ -e "-0.5,1.0" -f jpg

# High-quality 4K video
cargo run --release -- -i photos/ -o video/ -f mp4 -r 24 -q 18 --resolution 4K

# Use specific number of threads for processing
cargo run --release -- -i photos/ -o video/ -f mp4 -t 8

# Crop to center 50% of the image
cargo run --release -- -i photos/ -o video/ --crop="50%:50%:25%:25%" -f mp4

# Crop from right side (remove 200 pixels from right)
cargo run --release -- -i photos/ -o video/ --crop="600:600:0:0" -f mp4

## Performance

Lapsify uses parallel processing to speed up image processing:

- **Auto-detection**: By default, uses all available CPU cores
- **Manual control**: Use `-t/--threads` to specify exact number of threads
- **Progress tracking**: Real-time progress updates during processing
- **Memory efficient**: Processes images in parallel without excessive memory usage

### Threading Examples

```bash
# Use auto-detected number of threads (recommended)
cargo run --release -- -i photos/ -o video/ -f mp4

# Use 4 threads specifically
cargo run --release -- -i photos/ -o video/ -f mp4 -t 4

# Use single thread (for debugging or low-resource systems)
cargo run --release -- -i photos/ -o video/ -f mp4 -t 1
```

## Supported Image Formats

- JPEG (.jpg, .jpeg)
- PNG (.png)
- TIFF (.tiff, .tif)
- BMP (.bmp)
- WebP (.webp)
- RAW formats (.raw, .cr2, .nef, .arw)

## License

MIT License - see LICENSE file for details.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## Roadmap

- [ ] GUI interface
- [ ] Batch processing with different settings
- [ ] Advanced color grading
- [ ] Motion detection and stabilization
- [ ] Cloud processing support
