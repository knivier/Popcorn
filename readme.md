# Welcome to Popcorn
I'm trying to create a simple yet intuitive kernel framework. It's still unstable and all versions will be marked as pre releases for some time.

Enjoy!

Build instructions:
1. Run build.sh on WSL via ./build.sh - If you are having issues with this, use dos2unix to convert the file; GitHub seems to like putting nonunix characters in its files or something
2. Select "build kernel". This will automatically erase and build the kernel for you. If you have missing dependencies, the build should tell you. You will need NASM, GCC, LD and QEMU. If you have build errors, it will not show the specific errors; use rbuild.sh to have a non-fancy version of the building system
3. Run the kernel using QEMU (option 3). You can modify the QEMU setup via "settings" where you can allocate custom memory and cores to the QEMU system.

Please note, build.sh is a unix only tool (some get arounds using Git Bash) and is WSL built. Please also note, build.sh is a complete builder tool that ships with this product and should auto install any dependancies that aren't on your system. If you are encountering bugs, they will not show up on build.sh; you will get an abstracted error code. Use "trymake".sh instead for all error codes listed (but less interactive).

If you are on Windows and don't want to use WSL (unrecommended), you can use build.ps1 inside PowerShell (5 or higher, requires external script enabling). All .ps1 build/make systems are untested.

Finally, all build systems are at your own risk. I'm not responsible for any installations not working correctly nor any inconfigurations nor any other errors that may occur. This software is provided AS-IS without warrenties as shown in the LICENSE (please see that). If you are on a version below v1 (so, 0.1, 0.2, 0.3, etc) you are using a trial version. 

Assembly instructions for linux-based systems (or WSL) without a build maker are no longer provided due to the annoyance of keeping notes UTD. They are, therefore, not being included in this readme. Look through the trymake.sh to find out how you can implement it yourself instead.

## Project Overview
Popcorn is a simple kernel framework designed to help you understand the basics of operating system development. The project includes a minimal bootloader, a kernel written in C, and some basic assembly code to get things started.

### Directory Structure

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
        
        │   └── keyboard_map.h
        
        └── obj/              (Compiled objects)


### Building the Project
To build the project, follow the assembly and compilation instructions provided above. Ensure you have the necessary tools like `nasm`, `gcc`, and `ld` installed on your system.

### Running the Kernel
After building the project, you can run the kernel using QEMU with the command provided above. This will start the system and display the kernel messages on the screen. "Pops" are modular components that extend kernel functionality without modifying the core kernel.c file.

### Architecture Overview
Popcorn v0.5 introduces a modern console system and modular architecture:

- Console System: A complete VGA text mode abstraction layer that handles all screen output, cursor management, and color styling. All display operations go through the console system for consistency.

- Pop Modules: Self-contained modules that register with the kernel and can display information or provide functionality. Each pop module saves and restores console state to avoid interfering with user input.

- Utilities: Shared helper functions (like delay) used across multiple modules to reduce code duplication.

- Keyboard Input: Handled through interrupt-driven system with proper IDT setup and keyboard controller initialization.

### Future Plans
The project is still in its early stages and marked as unstable. Future updates will include more features and improvements to the kernel framework.

### Contributions
Thank you to Swarnim A for showing me this intel book that made inputs possible for this kernel. 
