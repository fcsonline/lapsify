# Design Document

## Overview

The lapsify-gui is a desktop application built with Rust and the eframe/egui framework that provides a graphical interface for the existing lapsify CLI tool. The application features a three-pane layout: a main image viewer, a settings sidebar, and a bottom carousel for image thumbnails. The design emphasizes usability, performance, and seamless integration with the existing lapsify functionality.

## Architecture

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    lapsify-gui Application                   │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────────┐  ┌─────────────────┐  │
│  │   Sidebar   │  │   Main Viewer   │  │    Carousel     │  │
│  │  Settings   │  │     Pane        │  │     Pane        │  │
│  │    Panel    │  │                 │  │                 │  │
│  └─────────────┘  └─────────────────┘  └─────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│                    Core Application Layer                   │
│  ┌─────────────┐  ┌─────────────────┐  ┌─────────────────┐  │
│  │   Image     │  │    Settings     │  │   Processing    │  │
│  │  Manager    │  │    Manager      │  │    Engine       │  │
│  └─────────────┘  └─────────────────┘  └─────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│                     System Integration                      │
│  ┌─────────────┐  ┌─────────────────┐  ┌─────────────────┐  │
│  │ File System │  │   lapsify CLI   │  │     eframe      │  │
│  │   Access    │  │   Integration   │  │   Framework     │  │
│  └─────────────┘  └─────────────────┘  └─────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Technology Stack

- **Framework**: eframe/egui for cross-platform GUI
- **Image Processing**: image crate for loading and displaying images
- **File System**: std::fs and rfd for file dialogs
- **Threading**: tokio for async operations and image loading
- **CLI Integration**: std::process::Command for executing lapsify CLI

## Components and Interfaces

### 1. Application State (`AppState`)

Central state management for the entire application:

```rust
pub struct AppState {
    pub selected_folder: Option<PathBuf>,
    pub images: Vec<ImageInfo>,
    pub selected_image_index: Option<usize>,
    pub settings: LapsifySettings,
    pub processing_status: ProcessingStatus,
    pub ui_state: UiState,
}

pub struct ImageInfo {
    pub path: PathBuf,
    pub thumbnail: Option<egui::TextureHandle>,
    pub full_image: Option<egui::TextureHandle>,
    pub metadata: ImageMetadata,
}
```

### 2. Settings Manager (`LapsifySettings`)

Manages all lapsify CLI parameters with validation:

```rust
pub struct LapsifySettings {
    // Image adjustments
    pub exposure: Vec<f32>,
    pub brightness: Vec<f32>,
    pub contrast: Vec<f32>,
    pub saturation: Vec<f32>,
    
    // Crop and positioning
    pub crop: Option<String>,
    pub offset_x: Vec<f32>,
    pub offset_y: Vec<f32>,
    
    // Output settings
    pub format: String,
    pub fps: u32,
    pub quality: u32,
    pub resolution: Option<String>,
    
    // Processing settings
    pub threads: usize,
    pub start_frame: Option<usize>,
    pub end_frame: Option<usize>,
}
```

### 3. Image Manager (`ImageManager`)

Handles image loading, caching, and thumbnail generation:

```rust
pub struct ImageManager {
    pub images: Vec<ImageInfo>,
    pub thumbnail_cache: HashMap<PathBuf, egui::TextureHandle>,
    pub full_image_cache: LruCache<PathBuf, egui::TextureHandle>,
}

impl ImageManager {
    pub async fn load_folder(&mut self, path: PathBuf) -> Result<(), ImageError>;
    pub async fn load_thumbnail(&mut self, path: &Path) -> Result<egui::TextureHandle, ImageError>;
    pub async fn load_full_image(&mut self, path: &Path) -> Result<egui::TextureHandle, ImageError>;
}
```

### 4. UI Components

#### Sidebar Panel (`SidebarPanel`)
- Settings input widgets for all lapsify parameters
- Real-time validation and error display
- Preset management for common configurations
- Export/import settings functionality

#### Main Viewer Panel (`MainViewerPanel`)
- Image display with zoom and pan capabilities
- Fit-to-window and actual-size viewing modes
- Image metadata display
- Navigation controls (previous/next)

#### Carousel Panel (`CarouselPanel`)
- Horizontal scrollable thumbnail strip
- Selection highlighting
- Lazy loading of thumbnails
- Drag-and-drop reordering support

### 5. Processing Engine (`ProcessingEngine`)

Integrates with the lapsify CLI for actual processing:

```rust
pub struct ProcessingEngine {
    pub current_job: Option<ProcessingJob>,
    pub progress_receiver: Option<Receiver<ProcessingProgress>>,
}

pub struct ProcessingJob {
    pub input_folder: PathBuf,
    pub output_folder: PathBuf,
    pub settings: LapsifySettings,
    pub status: JobStatus,
}
```

## Data Models

### Image Data Flow

```mermaid
graph TD
    A[Folder Selection] --> B[Scan Directory]
    B --> C[Filter Image Files]
    C --> D[Sort by Name/Date]
    D --> E[Generate Thumbnails]
    E --> F[Display in Carousel]
    F --> G[User Selects Image]
    G --> H[Load Full Resolution]
    H --> I[Display in Main Pane]
```

### Settings Data Model

The settings model mirrors the lapsify CLI arguments structure:

- **Value Arrays**: Support for single values or comma-separated arrays for animation
- **Validation**: Real-time validation with visual feedback
- **Persistence**: Save/load settings to/from JSON files
- **Presets**: Common configurations for quick access

### Processing Pipeline

```mermaid
graph LR
    A[UI Settings] --> B[Validate Parameters]
    B --> C[Generate CLI Command]
    C --> D[Execute lapsify CLI]
    D --> E[Monitor Progress]
    E --> F[Display Results]
```

## Error Handling

### Error Categories

1. **File System Errors**
   - Invalid folder selection
   - Permission issues
   - Missing files

2. **Image Processing Errors**
   - Unsupported formats
   - Corrupted files
   - Memory limitations

3. **Settings Validation Errors**
   - Out-of-range values
   - Invalid format strings
   - Incompatible parameter combinations

4. **CLI Integration Errors**
   - lapsify executable not found
   - CLI execution failures
   - Output parsing errors

### Error Display Strategy

- **Non-blocking notifications** for minor issues
- **Modal dialogs** for critical errors requiring user action
- **Inline validation** for settings with immediate feedback
- **Progress indicators** with cancellation options for long operations

## Testing Strategy

### Unit Testing

- **Settings validation logic** with comprehensive parameter ranges
- **Image loading and caching** with various file formats
- **CLI command generation** with different parameter combinations
- **Error handling** for all identified error scenarios

### Integration Testing

- **End-to-end workflow** from folder selection to video generation
- **CLI integration** with actual lapsify executable
- **File system operations** with various folder structures
- **Memory management** with large image sets

### UI Testing

- **Layout responsiveness** across different window sizes
- **User interaction flows** for common use cases
- **Accessibility** with keyboard navigation and screen readers
- **Performance** with large image collections

### Test Data

- **Sample image sets** with various formats and sizes
- **Edge cases** including empty folders, single images, and large collections
- **Invalid inputs** for robust error handling validation
- **Performance benchmarks** with realistic data sets

## Performance Considerations

### Image Loading Strategy

- **Lazy loading** of thumbnails as they become visible
- **Background loading** of full-resolution images
- **LRU cache** for full images to manage memory usage
- **Async loading** to prevent UI blocking

### Memory Management

- **Thumbnail size limits** (e.g., 200x200 pixels maximum)
- **Cache size limits** based on available system memory
- **Garbage collection** of unused textures
- **Progressive loading** for very large image sets

### UI Responsiveness

- **Frame rate targeting** (60 FPS for smooth interactions)
- **Async operations** for all file system and processing tasks
- **Progress indicators** for operations taking >100ms
- **Cancellation support** for long-running operations

## Security Considerations

### File System Access

- **Sandboxed file access** through system dialogs
- **Path validation** to prevent directory traversal
- **Permission checking** before file operations
- **Safe temporary file handling** for processing

### CLI Integration

- **Parameter sanitization** to prevent command injection
- **Output validation** from CLI execution
- **Error message filtering** to prevent information disclosure
- **Process isolation** for CLI execution