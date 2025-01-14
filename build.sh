#!/bin/bash

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

# Directory structure
SRC_DIR="src"
OBJ_DIR="src/obj"

# Compiler flags
CFLAGS="-m32 -c -ffreestanding -O2 -Wall -Wextra -I$SRC_DIR"
LDFLAGS="-m elf_i386 -T $SRC_DIR/link.ld"

# Source files
ASM_FILES="kernel.asm"
C_FILES="kernel.c pop_module.c shimjapii_pop.c" # Show all C files here

# Function to print status
print_status() {
    if [ $1 -eq 0 ]; then
        echo -e "${GREEN}✓ $2${NC}"
    else
        echo -e "${RED}✗ $2${NC}"
        exit 1
    fi
}

# Create obj directory if it doesn't exist
mkdir -p $OBJ_DIR

# Clean old object files
echo "Cleaning old files..."
rm -f $OBJ_DIR/*.o
rm -f kernel
print_status $? "Cleaned old files"

echo "Building Popcorn Kernel..."

# Assemble ASM files
for file in $ASM_FILES; do
    echo "Assembling $file..."
    nasm -f elf32 $SRC_DIR/$file -o $OBJ_DIR/${file%.asm}.o
    print_status $? "Assembled $file"
done

# Compile C files
for file in $C_FILES; do
    echo "Compiling $file..."
    gcc $CFLAGS $SRC_DIR/$file -o $OBJ_DIR/${file%.c}.o
    print_status $? "Compiled $file"
done

# Get all object files
OBJ_FILES=$(ls $OBJ_DIR/*.o)

# Link everything together
echo "Linking object files..."
ld $LDFLAGS -o kernel $OBJ_FILES
print_status $? "Linked kernel executable"

# Run in QEMU if linking was successful
if [ -f kernel ]; then
    echo "Starting QEMU..."
    qemu-system-i386 -cpu qemu32 -kernel src/kernel
    print_status $? "QEMU execution"
else
    echo -e "${RED}Kernel binary not found${NC}"
    exit 1
fi