#!/bin/bash

# Define colors and styles
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

# Configuration variables
OBJ_DIR="obj"
BUILD_BASE="buildbase"
BUILD_LOG="$BUILD_BASE/build.log"
CONFIG_FILE="$BUILD_BASE/.build_config"
QEMU_MEMORY="256"
QEMU_CORES="1"

# Check for required tools
check_dependencies() {
    local DEPS=("nasm" "gcc" "ld" "qemu-system-i386" "dialog")
    local MISSING=()
    
    for dep in "${DEPS[@]}"; do
        if ! command -v "$dep" >/dev/null 2>&1; then
            MISSING+=("$dep")
        fi
    done
    
    if [ ${#MISSING[@]} -ne 0 ]; then
        echo -e "${RED}Error: Missing dependencies: ${MISSING[*]}${NC}"
        echo "Please install the missing packages and try again."
        exit 1
    fi
}

# Load configuration
load_config() {
    if [ -f "$CONFIG_FILE" ]; then
        source "$CONFIG_FILE"
    fi
}

# Save configuration
save_config() {
    mkdir -p "$BUILD_BASE"
    echo "QEMU_MEMORY=$QEMU_MEMORY" > "$CONFIG_FILE"
    echo "QEMU_CORES=$QEMU_CORES" >> "$CONFIG_FILE"
}

# Logger function
log() {
    local level=$1
    local message=$2
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    mkdir -p "$BUILD_BASE"
    echo "[$timestamp] [$level] $message" >> "$BUILD_LOG"
    
    case $level in
        "INFO")  echo -e "${BLUE}$message${NC}" ;;
        "SUCCESS") echo -e "${GREEN}$message${NC}" ;;
        "ERROR") echo -e "${RED}$message${NC}" ;;
        "WARNING") echo -e "${YELLOW}$message${NC}" ;;
    esac
}

# Enhanced status checker
check_status() {
    local status=$?
    local operation=$1
    
    if [ $status -eq 0 ]; then
        log "SUCCESS" "$operation completed successfully"
    else
        log "ERROR" "$operation failed with status $status"
        if [ -f "$BUILD_LOG" ]; then
            dialog --title "Build Error" --textbox "$BUILD_LOG" 20 70
        fi
        exit 1
    fi
}

# Clean build files
clean_build() {
    log "INFO" "Cleaning build files..."
    rm -rf "$OBJ_DIR"
    rm -f kernel
    rm -f "$BUILD_LOG"
    check_status "Cleaning build files"
}

# Compile function
compile_file() {
    local file=$1
    local output=$2
    local type=$3
    
    log "INFO" "Compiling $file..."
    
    case $type in
        "asm")
            nasm -f elf32 "$file" -o "$output"
            ;;
        "c")
            gcc -m32 -c "$file" -o "$output" -Wall -Wextra
            ;;
    esac
    
    check_status "Compiling $file"
}

# Build function
build_kernel() {
    # Create obj directory if it doesn't already exist
    mkdir -p "$OBJ_DIR"

    # Compile assembly files
    compile_file "kernel.asm" "$OBJ_DIR/kasm.o" "asm"
    compile_file "idt.asm" "$OBJ_DIR/idt.o" "asm"
    
    # Compile C files
    compile_file "kernel.c" "$OBJ_DIR/kc.o" "c"
    compile_file "pop_module.c" "$OBJ_DIR/pop_module.o" "c"
    compile_file "shimjapii_pop.c" "$OBJ_DIR/shimjapii_pop.o" "c"
    compile_file "input_init.c" "$OBJ_DIR/input_init.o" "c"
    compile_file "spinner_pop.c" "$OBJ_DIR/spinner_pop.o" "c"  # Added spinner_pop.c
    
    # Link files
    log "INFO" "Linking object files..."
    ld -m elf_i386 -T link.ld -o kernel \
        "$OBJ_DIR"/kasm.o \
        "$OBJ_DIR"/kc.o \
        "$OBJ_DIR"/pop_module.o \
        "$OBJ_DIR"/shimjapii_pop.o \
        "$OBJ_DIR"/idt.o \
        "$OBJ_DIR"/input_init.o \
        "$OBJ_DIR"/spinner_pop.o  # Added spinner_pop.o to linking
    
    check_status "Linking object files"
}

# Run QEMU
run_qemu() {
    log "INFO" "Starting QEMU with ${QEMU_MEMORY}MB RAM and ${QEMU_CORES} cores..."
    qemu-system-i386 \
        -kernel kernel \
        -m "$QEMU_MEMORY" \
        -smp "$QEMU_CORES" \
        -monitor stdio
    
    check_status "Running QEMU"
}

# Show build logs
show_logs() {
    if [ -f "$BUILD_LOG" ]; then
        dialog --title "Build Logs" --textbox "$BUILD_LOG" 20 70
    else
        dialog --title "Error" --msgbox "No build logs found!" 8 40
    fi
}

# Settings menu
settings_menu() {
    while true; do
        local choice=$(dialog --title "Settings" \
            --menu "Configure build options:" 15 60 5 \
            1 "Set QEMU Memory (Current: ${QEMU_MEMORY}MB)" \
            2 "Set QEMU Cores (Current: ${QEMU_CORES})" \
            3 "Save Settings" \
            4 "Back to Main Menu" \
            2>&1 >/dev/tty)
        
        case $choice in
            1)
                local mem=$(dialog --inputbox "Enter memory in MB:" 8 40 "$QEMU_MEMORY" 2>&1 >/dev/tty)
                if [[ $mem =~ ^[0-9]+$ ]]; then
                    QEMU_MEMORY=$mem
                fi
                ;;
            2)
                local cores=$(dialog --inputbox "Enter number of cores:" 8 40 "$QEMU_CORES" 2>&1 >/dev/tty)
                if [[ $cores =~ ^[0-9]+$ ]]; then
                    QEMU_CORES=$cores
                fi
                ;;
            3)
                save_config
                dialog --title "Success" --msgbox "Settings saved!" 8 40
                ;;
            4|"")
                break
                ;;
        esac
    done
}

# Main menu
main_menu() {
    while true; do
        local choice=$(dialog --title "Popcorn Build System - Ultra" \
            --menu "Choose an operation:" 15 60 6 \
            1 "Build Kernel" \
            2 "Clean Build Files" \
            3 "Run in QEMU" \
            4 "Show Build Logs" \
            5 "Settings" \
            6 "Exit" \
            2>&1 >/dev/tty)
        
        case $choice in
            1)
                dialog --infobox "Building kernel..." 3 20
                clean_build
                build_kernel
                dialog --title "Success" --msgbox "Kernel built successfully!" 8 40
                ;;
            2)
                dialog --infobox "Cleaning build files..." 3 25
                clean_build
                dialog --title "Success" --msgbox "Build files cleaned!" 8 40
                ;;
            3)
                clear
                run_qemu
                ;;
            4)
                show_logs
                ;;
            5)
                settings_menu
                ;;
            6|"")
                clear
                exit 0
                ;;
        esac
    done
}

# Script entry point
check_dependencies
load_config
main_menu