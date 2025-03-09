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

Assembly instructions on WSL if you don't want to use the build systems (built off, custom)
### Assemble kernel.asm (if you have any assembly code)
nasm -f elf32 kernel.asm -o kasm.

### Compile shimjapii.c
gcc -m32 -c shimjapii.c -o shimjapii.o

### Compile kernel.c
gcc -m32 -c kernel.c -o kernel.o

### Link all object files together to create the final kernel executable
ld -m elf_i386 -T link.ld -o kernel kasm.o kernel.o shimjapii.o

### Start the system
qemu-system-i386 -kernel kernel

## Project Overview
Popcorn is a simple kernel framework designed to help you understand the basics of operating system development. The project includes a minimal bootloader, a kernel written in C, and some basic assembly code to get things started.

### Directory Structure

(taken from Gitingest, not UTD for 0.3+)
ROOT
└── knivier-popcorn/
    ├── readme.md
    ├── LICENSE
    ├── cmdlet.txt
    ├── pop.md
    ├── roadmap.md
    └── src/
        ├── build.sh
        ├── halt_pop.c
        ├── idt.asm
        ├── kernel
        ├── kernel.asm
        ├── kernel.c
        ├── keyboard_map.h
        ├── link.ld
        ├── pop_module.c
        ├── rbuild.sh
        ├── shimjapii_pop.c
        ├── spinner_pop.c
        ├── uptime_pop.c
        ├── buildbase/
        │   └── .build_config
        ├── includes/
        │   ├── pop_module.h
        │   └── spinner_pop.h
        └── obj/


### Building the Project
To build the project, follow the assembly and compilation instructions provided above. Ensure you have the necessary tools like `nasm`, `gcc`, and `ld` installed on your system.

### Running the Kernel
After building the project, you can run the kernel using QEMU with the command provided above. This will start the system and display the kernel messages on the screen. "Pops" are other files that run that aren't kernel.c based

### Future Plans
The project is still in its early stages and marked as unstable. Future updates will include more features and improvements to the kernel framework.

### Contributions
Thank you to Swarnim A for showing me this intel book that made inputs possible for this kernel. 