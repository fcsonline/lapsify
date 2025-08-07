
![Lapsify Logo](logo.png)

# Lapsify

A time-lapse image processor with adjustable parameters, available as both a command-line tool and a Qt-based GUI application.

## Features

### Command Line Interface
- Process time-lapse images with adjustable parameters
- Support for exposure, brightness, contrast, and saturation adjustments
- Cropping and offset controls
- Video output with customizable frame rate and quality
- Multi-threaded processing for improved performance

### GUI Application
- Modern Qt-based interface for macOS
- Real-time image preview with parameter adjustments
- Image carousel with keyboard navigation
- Sidebar with sliders for all Lapsify parameters
- Live preview of parameter changes
- Support for loading images from folders

## Installation

### Prerequisites

1. **Qt Development Libraries** (for GUI):
   ```bash
   # On macOS with Homebrew
   brew install qt@6
   
   # Or install Qt from https://www.qt.io/download
   ```

2. **FFmpeg** (for video processing):
   ```bash
   # On macOS with Homebrew
   brew install ffmpeg
   ```

### Building

1. **Command Line Version**:
   ```bash
   cargo build --release --bin lapsify
   ```

2. **GUI Version**:
   ```bash
   cargo build --release --bin lapsify-gui
   ```

## Usage

### Command Line Interface

```bash
# Basic usage
./target/release/lapsify -i /path/to/images -o /path/to/output

# With custom parameters
./target/release/lapsify \
  -i /path/to/images \
  -o /path/to/output \
  --exposure "0.0,1.5,-0.5" \
  --brightness "0,20,-10" \
  --contrast "1.0,1.5,0.8" \
  --saturation "1.0,1.8,0.5" \
  --crop "80%:60%:10%:20%" \
  --offset-x "0,20,0,-20" \
  --offset-y "0,10,0,-10" \
  --fps 30 \
  --quality 20 \
  --resolution 1920x1080
```

### GUI Application

```bash
# Run the GUI application
./target/release/lapsify-gui
```

#### GUI Features:
- **Load Images**: Click "Load Images" to select a folder containing your time-lapse images
- **Navigation**: Use "Previous" and "Next" buttons or arrow keys to navigate through images
- **Parameter Adjustment**: Use the sliders and controls in the right sidebar to adjust:
  - **Exposure**: -3.0 to +3.0 EV stops
  - **Brightness**: -100 to +100
  - **Contrast**: 0.1x to 3.0x multiplier
  - **Saturation**: 0.0x to 2.0x multiplier
  - **Crop Settings**: Enable/disable cropping with width, height, and position controls
  - **Frame Offset**: X and Y offset controls for stabilization or panning effects

#### Keyboard Shortcuts:
- **Left Arrow**: Previous image
- **Right Arrow**: Next image
- **Space**: Play/pause auto-advance (planned feature)

## Parameters

### Image Adjustments
- **Exposure**: Adjust exposure in EV stops (-3.0 to +3.0)
- **Brightness**: Add or subtract brightness (-100 to +100)
- **Contrast**: Multiply contrast (0.1 to 3.0, 1.0 = no change)
- **Saturation**: Multiply saturation (0.0 to 2.0, 1.0 = no change)

### Cropping
- **Crop String Format**: `width:height:x:y`
  - Values can be pixels or percentages (e.g., "80%:60%:10%:20%")
  - Negative X/Y values are percentages from right/bottom
- **Offset Controls**: Fine-tune crop position with pixel offsets

### Video Output
- **Frame Rate**: 1-120 fps (default: 24)
- **Quality**: CRF 0-51 (lower = better, 18-28 recommended)
- **Resolution**: Custom or preset (4K, HD, 720p)

## Examples

### Basic Time-lapse Processing
```bash
./target/release/lapsify -i ./timelapse_images -o ./output -f mp4
```

### Advanced Processing with Panning Effect
```bash
./target/release/lapsify \
  -i ./timelapse_images \
  -o ./output \
  --crop "90%:90%:5%:5%" \
  --offset-x "0,50,100,50,0" \
  --offset-y "0,25,0,-25,0" \
  --exposure "0.0,0.5,0.0" \
  --fps 30 \
  --quality 18
```

### Stabilization Effect
```bash
./target/release/lapsify \
  -i ./timelapse_images \
  -o ./output \
  --crop "95%:95%:2.5%:2.5%" \
  --offset-x "0,5,-5,0" \
  --offset-y "0,-3,3,0" \
  --fps 24
```

## Supported Formats

### Input Formats
- JPEG, PNG, TIFF, BMP, WebP
- RAW formats: CR2, NEF, ARW

### Output Formats
- **Images**: JPG, PNG, TIFF
- **Video**: MP4, MOV, AVI

## Performance

- Multi-threaded processing using all available CPU cores
- Memory-efficient processing for large image sequences
- GPU acceleration planned for future releases

## Development

### Project Structure
```
lapsify/
├── src/
│   ├── main.rs      # Command-line interface
│   └── gui.rs       # Qt GUI application
├── Cargo.toml       # Dependencies and build configuration
├── build.rs         # Build script for Qt bindings
└── README.md        # This file
```

### Building from Source
```bash
# Clone the repository
git clone https://github.com/fcsonline/lapsify.git
cd lapsify

# Build both versions
cargo build --release

# Run tests
cargo test

# Run the GUI
cargo run --bin lapsify-gui

# Run the CLI
cargo run --bin lapsify -- -i ./test_images -o ./output
```

## License

MIT License - see LICENSE file for details.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## Roadmap

- [ ] GPU acceleration for faster processing
- [ ] Auto-advance feature in GUI
- [ ] Batch processing in GUI
- [ ] Preset management
- [ ] Advanced stabilization algorithms
- [ ] Support for more video codecs
- [ ] Cross-platform GUI support (Windows, Linux)