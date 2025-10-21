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
    local DEPS=("nasm" "gcc" "ld" "qemu-system-x86_64" "dialog")
    local MISSING=()
    
    for dep in "${DEPS[@]}"; do
        if ! command -v "$dep" >/dev/null 2>&1; then
            MISSING+=("$dep")
        fi
    done
    
    if [ ${#MISSING[@]} -ne 0 ]; then
        echo -e "${YELLOW}Attempting to install missing dependencies: ${MISSING[*]}${NC}"
        for dep in "${MISSING[@]}"; do
            if command -v apt-get >/dev/null 2>&1; then
                sudo apt-get install -y "$dep"
            elif command -v yum >/dev/null 2>&1; then
                sudo yum install -y "$dep"
            elif command -v pacman >/dev/null 2>&1; then
                sudo pacman -S --noconfirm "$dep"
            else
                echo -e "${RED}Error: Package manager not found. Please install ${dep} manually.${NC}"
                exit 1
            fi
        done
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

# filepath: src/build.sh
# ...existing code...
check_status() {
    local status=$?
    local operation=$1
    local error_message=$2
    
    if [ $status -eq 0 ]; then
        log "SUCCESS" "$operation completed successfully"
    else
        log "ERROR" "$operation failed with status $status: $error_message"
        if [ -f "$BUILD_LOG" ]; then
            dialog --title "Build Error" --textbox "$BUILD_LOG" 20 70
        fi
        exit 1
    fi
}
# ...existing code...


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
            nasm -f elf64 "$file" -o "$output"
            ;;
        "c")
            gcc -m64 -c "$file" -o "$output" -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone
            ;;
    esac
    
    check_status "Compiling $file"
}

# Check for GRUB tools
check_grub() {
    if command -v grub2-mkrescue &> /dev/null; then
        return 0
    elif command -v grub-mkrescue &> /dev/null; then
        return 0
    else
        return 1
    fi
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
    compile_file "console.c" "$OBJ_DIR/console.o" "c"
    compile_file "utils.c" "$OBJ_DIR/utils.o" "c"
    compile_file "pop_module.c" "$OBJ_DIR/pop_module.o" "c"
    compile_file "shimjapii_pop.c" "$OBJ_DIR/shimjapii_pop.o" "c"
    compile_file "spinner_pop.c" "$OBJ_DIR/spinner_pop.o" "c" 
    compile_file "uptime_pop.c" "$OBJ_DIR/uptime_pop.o" "c"
    compile_file "halt_pop.c" "$OBJ_DIR/halt_pop.o" "c"
    compile_file "filesystem_pop.c" "$OBJ_DIR/filesystem_pop.o" "c"
    compile_file "multiboot2.c" "$OBJ_DIR/multiboot2.o" "c"
    compile_file "sysinfo_pop.c" "$OBJ_DIR/sysinfo_pop.o" "c"
    compile_file "memory_pop.c" "$OBJ_DIR/memory_pop.o" "c"
    compile_file "cpu_pop.c" "$OBJ_DIR/cpu_pop.o" "c"
    compile_file "dolphin_pop.c" "$OBJ_DIR/dolphin_pop.o" "c"
    compile_file "timer.c" "$OBJ_DIR/timer.o" "c"
    compile_file "scheduler.c" "$OBJ_DIR/scheduler.o" "c"
    compile_file "memory.c" "$OBJ_DIR/memory.o" "c"
    compile_file "init.c" "$OBJ_DIR/init.o" "c"
    compile_file "syscall.c" "$OBJ_DIR/syscall.o" "c"
    
    # Link files
    log "INFO" "Linking object files..."
    
    # Check if all object files exist
    for obj in "$OBJ_DIR"/kasm.o "$OBJ_DIR"/kc.o "$OBJ_DIR"/console.o "$OBJ_DIR"/utils.o "$OBJ_DIR"/pop_module.o "$OBJ_DIR"/shimjapii_pop.o "$OBJ_DIR"/idt.o "$OBJ_DIR"/spinner_pop.o "$OBJ_DIR"/uptime_pop.o "$OBJ_DIR"/halt_pop.o "$OBJ_DIR"/filesystem_pop.o "$OBJ_DIR"/multiboot2.o "$OBJ_DIR"/sysinfo_pop.o "$OBJ_DIR"/memory_pop.o "$OBJ_DIR"/cpu_pop.o "$OBJ_DIR"/dolphin_pop.o "$OBJ_DIR"/timer.o "$OBJ_DIR"/scheduler.o "$OBJ_DIR"/memory.o "$OBJ_DIR"/init.o "$OBJ_DIR"/syscall.o; do
        if [ ! -f "$obj" ]; then
            log "ERROR" "Missing object file: $obj"
            exit 1
        fi
    done
    
    if [ ! -f "link.ld" ]; then
        log "ERROR" "Linker script link.ld not found."
        exit 1
    fi

    ld -m elf_x86_64 -T link.ld -o kernel \
        "$OBJ_DIR/kasm.o" \
        "$OBJ_DIR/kc.o" \
        "$OBJ_DIR/console.o" \
        "$OBJ_DIR/utils.o" \
        "$OBJ_DIR/pop_module.o" \
        "$OBJ_DIR/shimjapii_pop.o" \
        "$OBJ_DIR/idt.o" \
        "$OBJ_DIR/spinner_pop.o" \
        "$OBJ_DIR/uptime_pop.o" \
        "$OBJ_DIR/halt_pop.o" \
        "$OBJ_DIR/filesystem_pop.o" \
        "$OBJ_DIR/multiboot2.o" \
        "$OBJ_DIR/sysinfo_pop.o" \
        "$OBJ_DIR/memory_pop.o" \
        "$OBJ_DIR/cpu_pop.o" \
        "$OBJ_DIR/dolphin_pop.o" \
        "$OBJ_DIR/timer.o" \
        "$OBJ_DIR/scheduler.o" \
        "$OBJ_DIR/memory.o" \
        "$OBJ_DIR/init.o" \
        "$OBJ_DIR/syscall.o"
    
    check_status "Linking object files"
}

# Create bootable ISO
create_iso() {
    if [ ! -f "kernel" ]; then
        log "ERROR" "Kernel file not found. Please build the kernel first."
        return 1
    fi
    
    log "INFO" "Creating bootable ISO..."
    
    # Check for GRUB
    GRUB_MKRESCUE=""
    if command -v grub2-mkrescue &> /dev/null; then
        GRUB_MKRESCUE="grub2-mkrescue"
    elif command -v grub-mkrescue &> /dev/null; then
        GRUB_MKRESCUE="grub-mkrescue"
    else
        log "ERROR" "grub-mkrescue or grub2-mkrescue not found."
        log "INFO" "Install on Fedora/RHEL: sudo dnf install grub2-tools-extra grub2-pc-modules xorriso mtools"
        log "INFO" "Install on Ubuntu/Debian: sudo apt-get install grub-pc-bin grub-common xorriso"
        return 1
    fi
    
    # Create ISO directory structure
    rm -rf isodir
    mkdir -p isodir/boot/grub
    
    # Copy kernel
    cp kernel isodir/boot/kernel
    check_status "Copying kernel"
    
    # Create GRUB configuration
    cat > isodir/boot/grub/grub.cfg << 'EOF'
set timeout=3
set default=0

menuentry "Popcorn Kernel x64" {
    echo "Loading Popcorn kernel..."
    multiboot2 /boot/kernel
    echo "Booting kernel..."
    boot
}
EOF
    
    # Create the ISO
    log "INFO" "Building ISO with $GRUB_MKRESCUE..."
    $GRUB_MKRESCUE -o popcorn.iso isodir >> "$BUILD_LOG" 2>&1
    
    if [ $? -ne 0 ]; then
        log "ERROR" "Failed to create ISO"
        rm -rf isodir
        return 1
    fi
    
    # Clean up
    rm -rf isodir
    
    if [ -f "popcorn.iso" ]; then
        log "SUCCESS" "ISO created successfully: popcorn.iso"
    else
        log "ERROR" "ISO file not found after creation"
        return 1
    fi
}

# Run QEMU with ISO
run_qemu() {
    if [ ! -f "popcorn.iso" ]; then
        log "WARNING" "ISO not found. Creating ISO first..."
        create_iso
        if [ $? -ne 0 ]; then
            return 1
        fi
    fi
    
    log "INFO" "Starting QEMU (64-bit) with ${QEMU_MEMORY}MB RAM and ${QEMU_CORES} cores..."
    log "INFO" "Press Ctrl+A then X to exit QEMU, or use the monitor console"
    
    qemu-system-x86_64 \
        -cdrom popcorn.iso \
        -cpu qemu64 \
        -m "$QEMU_MEMORY" \
        -smp "$QEMU_CORES"
    
    log "SUCCESS" "QEMU session ended"
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
        local choice=$(dialog --title "Popcorn Build System - x64 Edition" \
            --menu "Choose an operation:" 17 65 8 \
            1 "Build Kernel" \
            2 "Build Kernel + Create ISO" \
            3 "Create ISO (kernel must exist)" \
            4 "Run in QEMU (auto-creates ISO)" \
            5 "Clean Build Files" \
            6 "Show Build Logs" \
            7 "Settings" \
            8 "Exit" \
            2>&1 >/dev/tty)
        
        case $choice in
            1)
                dialog --infobox "Building kernel..." 3 25
                clean_build
                build_kernel
                dialog --title "Success" --msgbox "Kernel built successfully!\n\nNext: Create ISO to boot" 9 45
                ;;
            2)
                dialog --infobox "Building kernel and ISO..." 3 30
                clean_build
                build_kernel
                if [ $? -eq 0 ]; then
                    create_iso
                    if [ $? -eq 0 ]; then
                        dialog --title "Success" --msgbox "Kernel and ISO built successfully!\n\nReady to run in QEMU" 10 45
                    else
                        dialog --title "Error" --msgbox "Kernel built but ISO creation failed.\nCheck build logs for details." 10 45
                    fi
                else
                    dialog --title "Error" --msgbox "Kernel build failed.\nCheck build logs for details." 10 45
                fi
                ;;
            3)
                dialog --infobox "Creating bootable ISO..." 3 30
                create_iso
                if [ $? -eq 0 ]; then
                    dialog --title "Success" --msgbox "ISO created: popcorn.iso\n\nReady to run in QEMU" 9 40
                else
                    dialog --title "Error" --msgbox "ISO creation failed.\nCheck build logs for details." 10 40
                fi
                ;;
            4)
                clear
                run_qemu
                ;;
            5)
                dialog --infobox "Cleaning build files..." 3 30
                clean_build
                rm -f popcorn.iso
                dialog --title "Success" --msgbox "Build files and ISO cleaned!" 8 40
                ;;
            6)
                show_logs
                ;;
            7)
                settings_menu
                ;;
            8|"")
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