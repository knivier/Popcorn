# Welcome to Popcorn

A modern, modular 64-bit kernel framework designed for learning operating system development. Features a complete console system, text editor, system information tools, and more!

**Current Version: v0.5** - Now with Dolphin text editor, system info tools, and enhanced console features!

## Quick Start

1. **Build and Run**:
   ```bash
   cd src
   python3 buildmon.py
   # Click "Full Automation" for one-click build and run
   ```

2. **Try the Features**:
   - `help` - See all available commands
   - `sysinfo` - View system information
   - `mem -map` - See memory layout
   - `dol -new test.txt` - Create a text file
   - `ls` - List files and directories

3. **Text Editing**:
   - `dol -open filename.txt` - Open a file
   - Use arrow keys to navigate
   - Press ESC then `w` to save, `q` to quit

## Architecture

**Popcorn is now a 64-bit (x86-64) kernel!** This version has been fully converted from 32-bit to 64-bit architecture, featuring:
- Long mode initialization with page tables
- 64-bit GDT and IDT structures
- Support for modern x86-64 processors
- All the same features as before, now in 64-bit!

## Build Instructions

### GUI Build Tool (Easiest)

We provide a beautiful GUI build tool with modern animations:

```bash
cd src
python3 buildmon.py
```

Features:

- **Full Automation**: One-click build and run
- **Manual Mode**: Individual build steps with verbose output
- **Real-time Logs**: See build progress in real-time
- **Modern UI**: Dark theme with smooth animations
- **QEMU Integration**: Direct kernel testing

### Alternative: Interactive Build System

1. Run build.sh on WSL via `./build.sh` - If you are having issues with this, use dos2unix to convert the file; GitHub seems to like putting nonunix characters in its files or something

2. Select "build kernel". This will automatically erase and build the kernel for you. If you have missing dependencies, the build should tell you. You will need NASM, GCC, LD, QEMU (x86_64 version), and GRUB tools. If you have build errors, it will not show the specific errors; use trymake.sh to have a non-fancy version of the building system

**Note:** Direct kernel loading with `-kernel` may not work with all QEMU versions for 64-bit multiboot kernels. The ISO method is rhighly ecommended.

**Required tools for 64-bit build:**

- `nasm` (for 64-bit assembly)
- `gcc` with 64-bit support (`-m64`)
- `ld` with x86-64 support
- `qemu-system-x86_64` (64-bit QEMU)
- `grub-mkrescue`, `grub-pc-bin`, `xorriso` (for creating bootable ISO)

Install all dependencies:

**Fedora/RHEL:**

```bash
sudo dnf install nasm gcc qemu-system-x86 grub2-tools-extra grub2-pc-modules xorriso mtools
```

**Ubuntu/Debian:**

```bash
sudo apt-get install nasm gcc qemu-system-x86 grub-pc-bin grub-common xorriso
```

Please note, build.sh is a unix only tool (some get arounds using Git Bash) and is WSL built. Please also note, build.sh is a complete builder tool that ships with this product and should auto install any dependencies that aren't on your system. If you are encountering bugs, they will not show up on build.sh; you will get an abstracted error code. Use "trymake.sh" instead for all error codes listed (but less interactive).

If you are on Windows and don't want to use WSL (unrecommended), you can use build.ps1 inside PowerShell (5 or higher, requires external script enabling). All .ps1 build/make systems are untested and may need updating for 64-bit.

Finally, all build systems are at your own risk. I'm not responsible for any installations not working correctly nor any inconfigurations nor any other errors that may occur. This software is provided AS-IS without warranties as shown in the LICENSE (please see that). If you are on a version below v1 (so, 0.1, 0.2, 0.3, etc) you are using a trial version.

Assembly instructions for linux-based systems (or WSL) without a build maker are no longer provided due to the annoyance of keeping notes UTD. They are, therefore, not being included in this readme. Look through the trymake.sh to find out how you can implement it yourself instead.

## Project Overview

Popcorn is a simple kernel framework designed to help you understand the basics of operating system development. The project includes a minimal bootloader, a kernel written in C, and some basic assembly code to get things started.

### Directory Structure
```
ROOT

└── Popcorn/

    ├── readme.md
    
    ├── LICENSE
    
    ├── pop.md
    
    ├── roadmap.md
    
    └── src/
    
        ├── build.sh          (Interactive build system)
        
        ├── build.ps1         (PowerShell build script)
        
        ├── trymake.sh        (Simple build script)
        
        ├── kernel.c          (Main kernel)
        
        ├── kernel.asm        (Bootloader)
        
        ├── console.c         (Console system)
        
        ├── utils.c           (Shared utilities)
        
        ├── pop_module.c      (Pop module manager)
        
        ├── halt_pop.c        (Halt module)
        
        ├── spinner_pop.c     (Spinner animation)
        
        ├── uptime_pop.c      (Uptime counter)
        
        ├── filesystem_pop.c  (Filesystem module)
        
        ├── multiboot2.c      (Multiboot2 info parser)
        
        ├── sysinfo_pop.c     (System information)
        
        ├── memory_pop.c      (Memory management)
        
        ├── cpu_pop.c         (CPU information)
        
        ├── dolphin_pop.c     (Text editor)
        
        ├── shimjapii_pop.c   (Example pop)
        
        ├── idt.asm           (Interrupt descriptor)
        
        ├── link.ld           (Linker script)
        
        ├── buildbase/
        
        │   ├── .build_config
        
        │   └── build.log
        
        ├── includes/
        
        │   ├── console.h     (Console system header)
        
        │   ├── utils.h       (Utilities header)
        
        │   ├── pop_module.h  (Pop module header)
        
        │   ├── spinner_pop.h (Spinner header)
        
        │   ├── multiboot2.h  (Multiboot2 structures)
        
        │   ├── sysinfo_pop.h (System info header)
        
        │   ├── memory_pop.h  (Memory management header)
        
        │   ├── cpu_pop.h     (CPU info header)
        
        │   ├── dolphin_pop.h (Text editor header)
        
        │   └── keyboard_map.h
        
        └── obj/              (Compiled objects)
```

### Building the Project

To build the project, follow the assembly and compilation instructions provided above. Ensure you have the necessary tools like `nasm`, `gcc`, and `ld` installed on your system.

### Running the Kernel

After building the project, you can run the kernel using QEMU with the command provided above. This will start the system and display the kernel messages on the screen. "Pops" are modular components that extend kernel functionality without modifying the core kernel.c file.

## Features

### Core System
- **64-bit x86-64 Architecture**: Modern long mode with proper page tables
- **Multiboot2 Support**: Full bootloader specification compliance
- **Hardware Cursor**: Synchronized VGA hardware cursor positioning
- **Scrollback Buffer**: Page Up/Down to view command history
- **Command History**: Arrow keys for command navigation
- **Autocomplete**: Tab completion for all commands

### System Information Tools
- **`sysinfo`**: Complete system overview (kernel, CPU, memory, bootloader)
- **`mem -map`**: Extended memory map from Multiboot2
- **`mem -use`**: Memory usage statistics
- **`mem -stats`**: Detailed memory information
- **`cpu -hz`**: CPU frequency detection using RDTSC
- **`cpu -info`**: Detailed CPU information (vendor, features, brand string)

### File System
- **Complete Filesystem**: In-memory file system with directories
- **File Operations**: Create, read, write, delete, copy, search
- **Directory Management**: Create directories, navigate, list hierarchy
- **Path Support**: Full path resolution and navigation

### Dolphin Text Editor
- **Full-featured Editor**: Line-based text editing with cursor navigation
- **File Management**: Create, open, save, close text files
- **Navigation**: Arrow keys, Enter, Backspace with proper line handling
- **Command Mode**: ESC commands (w, q, wq, q!) like vim
- **Visual Feedback**: Line numbers, cursor position, modification status

### Console System
- **Modern UI**: Color-coded output with themes
- **Status Bar**: Real-time system information
- **Error Handling**: Standardized error messages
- **Double Buffering**: Smooth screen updates
- **Scrollback**: View previous output with Page Up/Down

### Build System
- **GUI Builder**: Modern Python GUI with animations
- **Automated Builds**: One-click compilation and ISO creation
- **Dependency Management**: Auto-installation of required tools
- **Cross-platform**: Works on Linux, WSL, and Windows

### Future Plan

The project is still in its early stages and marked as unstable. Future updates will include more features and improvements to the kernel framework.

### Contributions

Thank you to Swarnim A for showing me this intel book that made inputs possible for this kernel.
