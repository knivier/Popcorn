# Assemble kernel.asm to object file
nasm -f elf32 src/kernel.asm -o src/kasm.o
if [ $? -ne 0 ]; then
  echo "Error: Failed to assemble src/kernel.asm"
  exit 1
fi

# Compile shimjapii.c to object file
gcc -m32 -c src/shimjapii.c -o src/shimjapii.o
if [ $? -ne 0 ]; then
  echo "Error: Failed to compile src/shimjapii.c"
  exit 1
fi

# Compile kernel.c to object file
gcc -m32 -c src/kernel.c -o src/kernel.o
if [ $? -ne 0 ]; then
  echo "Error: Failed to compile src/kernel.c"
  exit 1
fi

# Link all object files together to create the kernel executable
ld -m elf_i386 -T src/link.ld -o kernel src/kasm.o src/kernel.o src/shimjapii.o
if [ $? -ne 0 ]; then
  echo "Error: Failed to link object files"
  exit 1
fi

# Run the system using QEMU
qemu-system-i386 -kernel kernel
if [ $? -ne 0 ]; then
  echo "Error: Failed to run the system"
  exit 1
fi
