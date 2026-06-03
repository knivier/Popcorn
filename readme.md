# Welcome to Popcorn

A modern, modular 64-bit kernel framework designed for learning operating system development. Features a complete console system, text editor, system information tools, and more!

**Current Version: v0.5** - Now with Dolphin text editor, system info tools, and enhanced console features!

## Quick Start

1. **Build and Run**:
   ```bash
   cd src
   python3 build/gui-tk.py
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
python3 build/gui-tk.py
```

Features:

- **Full Automation**: One-click build and run
- **Manual Mode**: Individual build steps with verbose output
- **Real-time Logs**: See build progress in real-time
- **Modern UI**: Dark theme with smooth animations
- **QEMU Integration**: Direct kernel testing

### Alternative: Shell Build Scripts

All build tooling lives under `src/build/`:

| Script | Platform | Description |
|--------|----------|-------------|
| `build/core.sh` | All | Unified CLI: kernel, UEFI img, ISO, QEMU tests |
| `build/macos.sh` | macOS | Delegates to `core.sh`; dialog menu when run with no args |
| `build/linux.sh` | Linux / Fedora | Delegates to `core.sh`; dialog menu when run with no args |
| `build/gui-macos.py` | macOS | WebView GUI (requires `pywebview`) |
| `build/gui-tk.py` | Cross-platform | Tkinter GUI with full automation |

**CLI (from `src/`):**
```bash
./build/core.sh build       # kernel only
./build/core.sh all         # kernel + UEFI loader + popcorn-uefi.img (flash this)
./build/core.sh test-uefi   # QEMU smoke tests
./build/core.sh run-uefi    # interactive QEMU with UEFI USB image
./build/core.sh iso         # legacy GRUB popcorn.iso
./build/core.sh run         # QEMU with legacy ISO
```

`./build/macos.sh` and `./build/linux.sh` accept the same commands as `core.sh`.

For verbose compile errors, run `./build/core.sh build` from a terminal or use the Tkinter GUI verbose build mode.

**Note:** Direct kernel loading with `-kernel` may not work with all QEMU versions for 64-bit multiboot kernels. The ISO method is highly recommended.

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

Please note, the shell build scripts are Unix-only tools. They auto-detect dependencies and can install missing packages on macOS (Homebrew) or Linux (dnf/apt/pacman). For detailed compiler output, use the CLI commands or the Tkinter GUI verbose build mode.

If you are on Windows without WSL, use the Tkinter GUI (`build/gui-tk.py`) where possible.

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
    
        ├── build/              (All build scripts and tooling)
        
        │   ├── core.sh         (Unified build entry point)
        
        │   ├── lib/            (kernel, UEFI, img, ISO, QEMU modules)
        
        │   ├── macos.sh        (macOS: core.sh + dialog menu)
        
        │   ├── linux.sh        (Linux: core.sh + dialog menu)
        
        │   ├── gui-macos.py    (macOS WebView GUI builder)
        
        │   ├── gui-tk.py       (Tkinter GUI builder)
        
        │   └── popcorn_build/  (Python build library + UI assets)
        
        ├── core/               (Kernel core: boot, scheduler, memory, …)
        
        ├── pops/               (Pop modules: console commands, Dolphin, …)
        
        ├── includes/           (Shared headers)
        
        ├── link.ld             (Linker script)
        
        ├── buildbase/          (Build logs and saved config; generated)
        
        └── obj/                (Compiled objects; generated)
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
