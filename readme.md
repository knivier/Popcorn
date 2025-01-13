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
