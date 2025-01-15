#!/bin/bash
# The simple builder
OBJ_DIR="obj"

# Clean build files
clean_build() {
    rm -rf "$OBJ_DIR"
    rm -f kernel
}

# Build function
build_kernel() {
    mkdir -p "$OBJ_DIR"

    # Compile assembly files
    nasm -f elf32 kernel.asm -o "$OBJ_DIR/kasm.o"
    if [ $? -ne 0 ]; then exit 1; fi
    
    nasm -f elf32 idt.asm -o "$OBJ_DIR/idt.o"
    if [ $? -ne 0 ]; then exit 1; fi
    
    # Compile C files
    gcc -m32 -c kernel.c -o "$OBJ_DIR/kc.o" -Wall -Wextra
    if [ $? -ne 0 ]; then exit 1; fi
    
    gcc -m32 -c pop_module.c -o "$OBJ_DIR/pop_module.o" -Wall -Wextra
    if [ $? -ne 0 ]; then exit 1; fi
    
    gcc -m32 -c shimjapii_pop.c -o "$OBJ_DIR/shimjapii_pop.o" -Wall -Wextra
    if [ $? -ne 0 ]; then exit 1; fi
    
    gcc -m32 -c input_init.c -o "$OBJ_DIR/input_init.o" -Wall -Wextra
    if [ $? -ne 0 ]; then exit 1; fi
    
    gcc -m32 -c spinner_pop.c -o "$OBJ_DIR/spinner_pop.o" -Wall -Wextra
    if [ $? -ne 0 ]; then exit 1; fi
    
    # Link files
    ld -m elf_i386 -T link.ld -o kernel \
        "$OBJ_DIR"/kasm.o \
        "$OBJ_DIR"/kc.o \
        "$OBJ_DIR"/pop_module.o \
        "$OBJ_DIR"/shimjapii_pop.o \
        "$OBJ_DIR"/idt.o \
        "$OBJ_DIR"/input_init.o \
        "$OBJ_DIR"/spinner_pop.o
}

clean_build
build_kernel