# Implementation Plan

-
  1. [x] Set up project structure and dependencies
  - Create new binary target for lapsify-gui in Cargo.toml
  - Add eframe, egui, image, tokio, rfd, and serde dependencies
  - Create main.rs entry point for GUI application
  - _Requirements: 6.1, 6.2_

-
  2. [x] Create basic eframe application structure
  - Implement eframe::App trait for main application
  - Set up basic window with title and sizing
  - Create placeholder content showing application purpose
  - _Requirements: 6.1, 6.2_

-
  3. [x] Implement core data structures and keyframe state management
  - Create AppState struct with keyframe settings instead of simple settings
  - Implement ImageInfo struct for image metadata and texture handles
  - Create KeyframeSettings and KeyframeData structs for parameter animation
  - Add GlobalSettings struct for non-keyframe parameters
  - Add serialization/deserialization for keyframe settings persistence
  - _Requirements: 4.1, 4.4, 4.5, 7.1, 8.1_

-
  4. [x] Set up three-pane layout with right-side settings panel
  - Replace CentralPanel with right SidePanel, CentralPanel, and TopBottomPanel
  - Create right-side sidebar for settings (disabled by default)
  - Create main viewer area with folder selection controls
  - Create bottom carousel panel for thumbnails
  - Implement basic pane resizing and proportions
  - _Requirements: 6.1, 6.2, 6.3, 6.5_

-
  5. [x] Implement folder selection functionality in main panel
  - Add file dialog integration using rfd crate
  - Create folder selection button in main viewer panel (not sidebar)
  - Implement folder path validation and display
  - Add error handling for invalid folder selection
  - Enable settings sidebar when folder is successfully loaded
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 4.2, 4.3_

-
  6. [x] Create image scanning and loading system
  - Implement directory scanning for supported image formats
  - Create image file filtering based on lapsify supported formats
  - Add chronological sorting of image files
  - Implement async image loading with error handling
  - _Requirements: 1.2, 7.1, 7.2, 7.3_

-
  7. [x] Build thumbnail generation and caching system
  - Implement thumbnail generation with size constraints (200x200)
  - Create LRU cache for thumbnail storage
  - Add async thumbnail loading with loading indicators
  - Implement memory management for thumbnail cache
  - _Requirements: 2.1, 2.5_

-
  8. [x] Create carousel panel with thumbnail display
  - Implement horizontal scrollable thumbnail strip in bottom pane
  - Add thumbnail click handling for selection
  - Create visual selection highlighting for active thumbnail
  - Implement lazy loading as thumbnails become visible
  - _Requirements: 2.1, 2.2, 2.3, 2.4_

-
  9. [x] Implement main image viewer panel
  - Create full-size image display in main pane
  - Add image scaling to fit pane while maintaining aspect ratio
  - Implement zoom and pan functionality for large images
  - Create placeholder display when no image is selected
  - _Requirements: 3.1, 3.2, 3.3, 3.4_

-
  10. [x] Build keyframe management controls in right sidebar
  - Create "Number of keyframes" slider (1-50, limited by image count)
  - Implement "Selected keyframe" selector control
  - Add keyframe controls at the top of the right sidebar
  - Disable all keyframe controls when no folder is loaded
  - Update keyframe data structure when number of keyframes changes
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 8.1, 8.2, 8.5_

- 10.1. [x] Build parameter input widgets for selected keyframe
  - Create input widgets for keyframe-specific parameters (exposure, brightness,
    contrast, saturation)
  - Implement per-keyframe parameter editing based on selected keyframe
  - Add offset_x, offset_y, zoom, and rotation controls for selected keyframe
  - Update UI to show values for currently selected keyframe
  - Store parameter changes only for the selected keyframe
  - _Requirements: 8.2, 8.3, 8.4_

- 10.2. [x] Add global settings controls in right sidebar
  - Create controls for non-keyframe parameters (format, fps, quality,
    resolution)
  - Add crop parameter input with validation
  - Implement processing settings (threads, start_frame, end_frame)
  - Position global settings below keyframe controls
  - _Requirements: 4.1, 4.2_

-
  11. [x] Add keyframe settings validation and error display
  - Implement real-time validation for all keyframe parameter ranges
  - Create visual error indicators for invalid keyframe inputs
  - Add validation messages matching lapsify CLI constraints
  - Implement parameter interdependency validation across keyframes
  - Validate keyframe count against available images
  - _Requirements: 4.4, 4.5, 7.4, 8.6_

-
  12. [x] Create CLI integration system with keyframe arrays
  - Implement command generation from keyframe settings struct
  - Build parameter arrays from keyframe data for CLI
  - Format array parameters with '=' syntax (e.g., --exposure=-2,1.2,4)
  - Add process execution for lapsify CLI with array parameter passing
  - Create output directory selection for processed results
  - Implement CLI output parsing and error handling
  - _Requirements: 5.1, 5.4, 4.8, 8.4_

-
  13. [x] Build progress tracking and cancellation
  - Implement progress indicator UI for time-lapse generation
  - Add cancellation button and process termination handling
  - Create progress updates from CLI execution monitoring
  - Display success/failure messages with output file location
  - _Requirements: 5.2, 5.3, 5.4_

-
  14. [x] Add responsive layout and window management
  - Implement pane resizing with minimum size constraints
  - Add window size persistence between sessions
  - Create responsive behavior for different screen sizes
  - Implement pane proportion adjustment and saving
  - _Requirements: 6.1, 6.2, 6.3, 6.4_

-
  15. [x] Implement keyframe settings persistence and presets
  - Add save/load functionality for keyframe settings to JSON files
  - Create keyframe presets for common animation configurations
  - Implement session state persistence for current folder and keyframe settings
  - Add import/export functionality for sharing keyframe configurations
  - Preserve keyframe data when reducing keyframe count
  - _Requirements: 4.6, 4.7, 8.6_

-
  16. [x] Add comprehensive error handling
  - Implement error display system with non-blocking notifications
  - Create modal dialogs for critical errors requiring user action
  - Add file system error handling with user-friendly messages
  - Implement graceful degradation for missing lapsify CLI
  - _Requirements: 1.3, 7.4_

-
  17. [x] Create keyboard navigation and accessibility
  - Add keyboard shortcuts for common actions (folder selection, image
    navigation)
  - Implement tab navigation through UI elements
  - Add screen reader support with proper labels
  - Create focus indicators for keyboard navigation
  - _Requirements: 6.1_

-
  18. [x] Implement performance optimizations
  - Add background loading for full-resolution images
  - Implement texture cleanup and memory management
  - Create efficient thumbnail loading with viewport culling
  - Add frame rate optimization for smooth UI interactions
  - _Requirements: 2.5, 3.4_

-
  19. [x] Add final integration and testing
  - Create end-to-end workflow testing from folder selection to video generation
  - Implement error scenario testing with various edge cases
  - Add performance testing with large image collections
  - Create integration tests with actual lapsify CLI execution
  - _Requirements: 5.1, 5.2, 5.3, 5.4_

-
  20. [ ] Move settings panel to right sidebar as per design
  - Change SidePanel::left to SidePanel::right for settings panel
  - Update responsive layout constraints for right-side positioning
  - Ensure folder selection controls remain in main panel
  - Fix pane proportions and minimum sizes for right-side layout
  - _Requirements: 6.1, 6.2, 6.3, 6.5_

-
  21. [ ] Implement proper keyframe-based parameter system
  - Current implementation uses simple arrays instead of proper keyframe system
  - Add KeyframeSettings and KeyframeData structs as per design
  - Implement "Number of keyframes" and "Selected keyframe" controls
  - Create per-keyframe parameter editing UI
  - Update CLI command generation to use keyframe arrays correctly
  - Add keyframe timeline visualization in the UI
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 8.1, 8.2, 8.3, 8.4, 8.5, 8.6_

-
  22. [ ] Add missing keyframe-specific parameters
  - Add zoom and rotation controls to keyframe parameters
  - Implement proper offset_x and offset_y keyframe controls
  - Update parameter validation for keyframe-specific ranges
  - Add keyframe interpolation preview functionality
  - _Requirements: 8.2, 8.3, 8.4_

-
  23. [ ] Add comprehensive testing and validation
  - Test end-to-end workflow with actual image folders
  - Validate CLI command generation with keyframe arrays
  - Test error handling with various edge cases
  - Verify performance with large image collections
  - Test session persistence and preset functionality
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 4.6, 4.7_
