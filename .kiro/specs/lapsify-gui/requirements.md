# Requirements Document

## Introduction

The lapsify-gui is a desktop application that provides a graphical user interface for the existing lapsify CLI command. The application will allow users to visually select folders, preview images, and configure time-lapse settings through an intuitive three-pane layout built with Rust and the eframe/egui framework.

## Requirements

### Requirement 1

**User Story:** As a user, I want to select a folder containing images through a file dialog, so that I can easily browse and choose the source directory for my time-lapse.

#### Acceptance Criteria

1. WHEN the user clicks a "Select Folder" button THEN the system SHALL open a native file dialog for folder selection
2. WHEN the user selects a valid folder THEN the system SHALL load all supported image files from that folder
3. IF the selected folder contains no supported image files THEN the system SHALL display an appropriate warning message
4. WHEN a folder is successfully loaded THEN the system SHALL display the folder path in the interface

### Requirement 2

**User Story:** As a user, I want to see thumbnail previews of all images in a carousel at the bottom of the application, so that I can quickly browse through my image sequence.

#### Acceptance Criteria

1. WHEN images are loaded from a folder THEN the system SHALL display thumbnails in chronological order in the bottom carousel pane
2. WHEN there are more thumbnails than can fit in the visible area THEN the system SHALL provide horizontal scrolling functionality
3. WHEN the user clicks on a thumbnail THEN the system SHALL highlight the selected thumbnail
4. WHEN the user clicks on a thumbnail THEN the system SHALL load the full-size image in the main pane
5. WHEN images are loading THEN the system SHALL show loading indicators for thumbnails

### Requirement 3

**User Story:** As a user, I want to view a selected image in full detail in the main pane, so that I can inspect the quality and content of individual frames.

#### Acceptance Criteria

1. WHEN a thumbnail is clicked THEN the system SHALL display the corresponding full-size image in the main pane
2. WHEN no image is selected THEN the main pane SHALL display a placeholder or welcome message
3. WHEN the main pane is resized THEN the system SHALL scale the image appropriately while maintaining aspect ratio
4. WHEN an image is too large for the main pane THEN the system SHALL provide zoom and pan functionality

### Requirement 4

**User Story:** As a user, I want to configure all lapsify settings through a sidebar panel, so that I can customize my time-lapse output without using command-line arguments.

#### Acceptance Criteria

1. WHEN the application starts THEN the system SHALL display a sidebar with all available lapsify configuration options
2. WHEN the user modifies a setting THEN the system SHALL validate the input and provide immediate feedback
3. WHEN invalid settings are entered THEN the system SHALL display clear error messages
4. WHEN settings are changed THEN the system SHALL persist the changes for the current session
5. IF lapsify has default values THEN the system SHALL initialize settings with those defaults

### Requirement 5

**User Story:** As a user, I want to generate a time-lapse from the selected images and configured settings, so that I can create my video output through the GUI.

#### Acceptance Criteria

1. WHEN the user clicks a "Generate Time-lapse" button THEN the system SHALL validate that a folder is selected and settings are valid
2. WHEN generation starts THEN the system SHALL display a progress indicator showing the current status
3. WHEN generation is in progress THEN the system SHALL allow the user to cancel the operation
4. WHEN generation completes successfully THEN the system SHALL display a success message and the output file location
5. IF generation fails THEN the system SHALL display the error message from the lapsify CLI

### Requirement 6

**User Story:** As a user, I want the application to have a responsive three-pane layout, so that I can efficiently use the interface on different screen sizes.

#### Acceptance Criteria

1. WHEN the application window is resized THEN the system SHALL maintain the three-pane layout with appropriate proportions
2. WHEN the window is too small THEN the system SHALL provide minimum sizes for each pane to remain functional
3. WHEN panes are resized THEN the system SHALL allow users to adjust the relative sizes of the sidebar and carousel
4. WHEN the application starts THEN the system SHALL remember the last window size and pane proportions

### Requirement 7

**User Story:** As a user, I want the application to handle various image formats supported by lapsify, so that I can work with my existing image collections.

#### Acceptance Criteria

1. WHEN scanning a folder THEN the system SHALL recognize all image formats supported by the lapsify CLI
2. WHEN unsupported files are present THEN the system SHALL ignore them without displaying errors
3. WHEN loading images THEN the system SHALL handle common formats including JPEG, PNG, TIFF, and others as supported by lapsify
4. IF image loading fails THEN the system SHALL display a placeholder thumbnail with an error indicator