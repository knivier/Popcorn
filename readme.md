# Welcome to Popcorn
I'm trying to create a simple yet intuitive kernel framework. It's still unstable and all versions will be marked as pre releases for some time.

Enjoy!

Assembly instructions on WSL (built off)
### Assemble kernel.asm (if you have any assembly code)
nasm -f elf32 kernel.asm -o kasm.o

### Compile shimjapii.c
gcc -m32 -c shimjapii.c -o shimjapii.o

### Compile kernel.c
gcc -m32 -c kernel.c -o kernel.o

### Link all object files together to create the final kernel executable
ld -m elf_i386 -T link.ld -o kernel kasm.o kernel.o shimjapii.o

# Start the system
qemu-system-i386 -kernel kernel

## Project Overview
Popcorn is a simple kernel framework designed to help you understand the basics of operating system development. The project includes a minimal bootloader, a kernel written in C, and some basic assembly code to get things started.

### Project Structure
- `kernel.asm`: Assembly code for the bootloader.
- `kernel.c`: Main kernel code written in C.
- `shimjapii.c` and `shimjapii.h`: Additional C code and header file for kernel functionality.
- `link.ld`: Linker script to define the memory layout of the kernel.
- `.vscode/settings.json`: VSCode settings for the project.

### Building the Project
To build the project, follow the assembly and compilation instructions provided above. Ensure you have the necessary tools like `nasm`, `gcc`, and `ld` installed on your system.

### Running the Kernel
After building the project, you can run the kernel using QEMU with the command provided above. This will start the system and display the kernel messages on the screen. "Pops" are other files that run that aren't kernel.c based

### Future Plans
The project is still in its early stages and marked as unstable. Future updates will include more features and improvements to the kernel framework.

### Contributions
Thank you to Swarnim A for showing me this intel book that made inputs possible for this kernel. 