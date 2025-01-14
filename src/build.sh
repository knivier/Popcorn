#!/bin/bash

# Define colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No color

# Directory for object files
OBJ_DIR="obj"

# Create the obj directory if it doesn't exist
mkdir -p $OBJ_DIR

# Function to check the status of the last command and display an appropriate message
check_status() {
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}Success${NC}: $1 completed successfully."
    else
        echo -e "${RED}Error${NC}: $1 failed."
        exit 1
    fi
}

# Start of the script
echo -e "${YELLOW}Starting the build process...${NC}"

# Clean old object files
echo -e "${YELLOW}Cleaning old object files...${NC}"
rm -f $OBJ_DIR/*.o
check_status "Cleaning object files"

# Compile the assembly files
echo -e "${YELLOW}Compiling kernel.asm...${NC}"
nasm -f elf32 kernel.asm -o $OBJ_DIR/kasm.o
check_status "Compiling kernel.asm"

echo -e "${YELLOW}Compiling idt.asm...${NC}"
nasm -f elf32 idt.asm -o $OBJ_DIR/idt.o
check_status "Compiling idt.asm"

# Compile the C files
echo -e "${YELLOW}Compiling kernel.c...${NC}"
gcc -m32 -c kernel.c -o $OBJ_DIR/kc.o
check_status "Compiling kernel.c"

echo -e "${YELLOW}Compiling pop_module.c...${NC}"
gcc -m32 -c pop_module.c -o $OBJ_DIR/pop_module.o
check_status "Compiling pop_module.c"

echo -e "${YELLOW}Compiling shimjapii_pop.c...${NC}"
gcc -m32 -c shimjapii_pop.c -o $OBJ_DIR/shimjapii_pop.o
check_status "Compiling shimjapii_pop.c"

echo -e "${YELLOW}Compiling input_init.c...${NC}"
gcc -m32 -c input_init.c -o $OBJ_DIR/input_init.o
check_status "Compiling input_init.c"

# Link all object files into a final executable
echo -e "${YELLOW}Linking object files...${NC}"
ld -m elf_i386 -T link.ld -o kernel $OBJ_DIR/kasm.o $OBJ_DIR/kc.o $OBJ_DIR/pop_module.o $OBJ_DIR/shimjapii_pop.o $OBJ_DIR/idt.o $OBJ_DIR/input_init.o
check_status "Linking object files"

# Run the kernel with QEMU
echo -e "${YELLOW}Running kernel in QEMU...${NC}"
qemu-system-i386 -kernel kernel
check_status "Running QEMU"
