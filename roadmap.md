
#Popcorn 
# Project Roadmap
## Overview
Popcorn is a modular and custom kernel framework for developers. It aims to provide a simple yet intuitive environment to understand the basics of operating system development. The project includes a minimal bootloader, a kernel written in C, and some basic assembly code to get things started.

## Completed Tasks
### Initial Setup and Basic Kernel
- Setup Project Structure
- Assembly Instructions on WSL
- Building the Project
- Running the Kernel

### Version 0.5 Improvements
- Implemented complete console system for VGA text mode management
- Created shared utilities module (utils.c) to reduce code duplication
- Updated all pop modules to use console system with cursor save/restore
- Fixed keyboard input handling to prevent duplicate processing
- Added proper backspace support
- Modernized display output with professional console appearance
- Improved status display layout (Ticks and Running indicators at top)
- x64 conversion, ISO creation can run natively on systems without VM

## Features
- **Console System**
  - Complete VGA text mode abstraction layer
  - Cursor management and positioning
  - Color styling with predefined themes
  - Screen scrolling and clearing
  - Status bar and header support
  
- **Pop Module System**
  - Ability to register and execute custom pop modules
  - Modular design to easily add new pops without modifying the existing code
  - All modules use console system for consistent behavior
  - Cursor state preservation to prevent input interference

- **Utilities Framework**
  - Shared helper functions across modules
  - Common delay function for animations
  - Reduces code duplication

## Current Pops
### Shimjapii Pop
- **Description**: Example pop module for developers
- **Location**: Displays in bottom right corner
- **Integration**: Uses console system with cursor save/restore

### Spinner Pop
- **Description**: Animated loading indicator
- **Location**: Top-right corner (row 0)
- **Display**: "Running... |/-\" with rotating animation
- **Integration**: Updates without interfering with user input

### Uptime Pop
- **Description**: System tick counter
- **Location**: Top-left corner (row 0)
- **Display**: "Ticks: XXXXXX"
- **Integration**: Real-time counter that preserves cursor position

### Halt Pop
- **Description**: Visual halt screen with animations
- **Features**: Green screen effect, rainbow colors, fade to black
- **Integration**: Uses shared util_delay function

### Filesystem Pop
- **Description**: In-memory filesystem with directories
- **Commands**: create, write, read, delete, mkdir, go, back, listsys
- **Integration**: Uses console system for status messages

## Pops in Progress

## Future Plans
- **Enhancements and Features**
  - Include more features and improvements to the kernel framework.
  - Add more pop modules to extend the kernel's functionality.
  - Improve the documentation and provide more detailed guides for developers.
  

## Work in Progress
- **Stabilize the Project**
  - Mark all versions as pre-releases until the project reaches stability.
  - Perform thorough testing and debugging to ensure reliability.
  - Create a working .iso file to boot into the kernel from a VM or T440p (zorinos only)

## Next Steps
- **Module Development**
  - Create and integrate additional pop modules using the console system
  - Ensure that each module follows the cursor save/restore pattern
  - All new pops should use shared utilities where appropriate
  - The ability to run C++ files to build the project in C++ but natively in C
  
- **Console Enhancements**
  - Add more color schemes and themes
  - Implement double-buffering for flicker-free animations
  - Add support for custom fonts or character sets
  
- **Community Engagement**
  - Encourage contributions from other developers
  - Provide clear guidelines for contributing and reporting issues
  - Maintain pop.md with updated examples for new developers

- **Documentation**
  - Improve and expand the documentation to cover the console system
  - Create tutorials and examples to help new developers get started with Popcorn
  - Document best practices for pop module development

## Long-term Goals
- **Advanced Features**
  - Implement advanced kernel features such as multitasking, memory management, and file systems.
  - Optimize the kernel for performance and scalability.

- **Release Management**
  - Transition from pre-releases to stable releases.
  - Establish a regular release cycle with planned features and improvements.

---

We welcome any contributions and feedback from the community to help improve Popcorn.