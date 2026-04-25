#!/usr/bin/env bash
set -Eeuo pipefail

GREEN=$'\033[0;32m'
RED=$'\033[0;31m'
YELLOW=$'\033[1;33m'
BLUE=$'\033[0;34m'
NC=$'\033[0m'

OBJ_DIR="${OBJ_DIR:-obj}"
BUILD_BASE="${BUILD_BASE:-buildbase}"
BUILD_LOG="${BUILD_LOG:-$BUILD_BASE/build.log}"
CONFIG_FILE="${CONFIG_FILE:-$BUILD_BASE/.build_config}"

QEMU_MEMORY="${QEMU_MEMORY:-256}"
QEMU_CORES="${QEMU_CORES:-1}"

TARGET_TRIPLE="x86_64-unknown-elf"
KERNEL_OUT="${KERNEL_OUT:-kernel}"
ISO_OUT="${ISO_OUT:-popcorn.iso}"

HOST_OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

mkdir -p "$BUILD_BASE"
: > "$BUILD_LOG" || true

log() {
  local level="$1"; shift
  local message="$*"
  local timestamp
  timestamp="$(date '+%Y-%m-%d %H:%M:%S')"
  printf '[%s] [%s] %s\n' "$timestamp" "$level" "$message" >>"$BUILD_LOG"
  case "$level" in
    INFO) printf '%s%s%s\n' "$BLUE" "$message" "$NC" ;;
    SUCCESS) printf '%s%s%s\n' "$GREEN" "$message" "$NC" ;;
    WARNING) printf '%s%s%s\n' "$YELLOW" "$message" "$NC" ;;
    ERROR) printf '%s%s%s\n' "$RED" "$message" "$NC" ;;
    *) printf '%s\n' "$message" ;;
  esac
}

die() {
  log ERROR "$*"
  log INFO "See logs at: $BUILD_LOG"
  exit 1
}

have() { command -v "$1" >/dev/null 2>&1; }

load_config() {
  if [[ -f "$CONFIG_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$CONFIG_FILE"
  fi
}

save_config() {
  mkdir -p "$BUILD_BASE"
  {
    printf 'QEMU_MEMORY=%q\n' "$QEMU_MEMORY"
    printf 'QEMU_CORES=%q\n' "$QEMU_CORES"
  } >"$CONFIG_FILE"
}

brew_install_if_missing() {
  local formula="$1"
  if brew list --formula "$formula" >/dev/null 2>&1; then
    return 0
  fi
  log INFO "Installing $formula via Homebrew..."
  brew install "$formula" >>"$BUILD_LOG" 2>&1 || die "Homebrew failed installing $formula"
}

check_dependencies() {
  local missing=()
  local deps=(nasm qemu-system-x86_64)

  for dep in "${deps[@]}"; do
    if ! have "$dep"; then
      missing+=("$dep")
    fi
  done

  if [[ ${#missing[@]} -eq 0 ]]; then
    return 0
  fi

  if [[ "$HOST_OS" == "darwin" ]]; then
    have brew || die "Homebrew is required on macOS. Install it, then re-run this script."
    for dep in "${missing[@]}"; do
      case "$dep" in
        nasm) brew_install_if_missing nasm ;;
        qemu-system-x86_64) brew_install_if_missing qemu ;;
        *) die "Missing dependency: $dep (install it and re-run)" ;;
      esac
    done
  else
    die "Missing dependencies: ${missing[*]}. Install them and re-run."
  fi
}

find_grub_mkrescue() {
  if have grub2-mkrescue; then
    printf '%s' "grub2-mkrescue"
    return 0
  fi
  if have grub-mkrescue; then
    printf '%s' "grub-mkrescue"
    return 0
  fi
  # Homebrew GRUB cross builds:
  # - BIOS (SeaBIOS/QEMU default): i686-elf-grub-mkrescue
  # - EFI-only images often come from x86_64-elf-grub-mkrescue
  if have i686-elf-grub-mkrescue; then
    printf '%s' "i686-elf-grub-mkrescue"
    return 0
  fi
  if have x86_64-elf-grub-mkrescue; then
    printf '%s' "x86_64-elf-grub-mkrescue"
    return 0
  fi
  return 1
}

ensure_elf_toolchain() {
  # We need an ELF-capable compiler + linker. Apple's default toolchain produces Mach-O.
  if have x86_64-elf-gcc && have x86_64-elf-ld; then
    export CC="x86_64-elf-gcc"
    export LD="x86_64-elf-ld"
    log INFO "Using cross toolchain: $CC + $LD"
    return 0
  fi

  if have clang; then
    export CC="clang"
  else
    die "No C compiler found (need clang or x86_64-elf-gcc)."
  fi

  if have ld.lld; then
    export LD="ld.lld"
  elif have x86_64-elf-ld; then
    export LD="x86_64-elf-ld"
  else
    if [[ "$HOST_OS" == "darwin" ]]; then
      have brew || die "Need an ELF linker. Install Homebrew and then install LLVM (lld)."
      log WARNING "Missing ELF linker. Installing LLVM (for ld.lld) via Homebrew..."
      brew_install_if_missing llvm
      local llvm_prefix
      llvm_prefix="$(brew --prefix llvm)"
      export PATH="$llvm_prefix/bin:$PATH"
      have ld.lld || die "ld.lld still not found after installing llvm. Ensure Homebrew's llvm bin is on PATH."
      export LD="ld.lld"
    else
      die "Need an ELF linker (ld.lld or x86_64-elf-ld)."
    fi
  fi

  log INFO "Using toolchain: $CC + $LD (target $TARGET_TRIPLE)"
}

clean_build() {
  log INFO "Cleaning build artifacts..."
  rm -rf "$OBJ_DIR" "$KERNEL_OUT" "$ISO_OUT" isodir 2>/dev/null || true
  rm -f "$BUILD_LOG" 2>/dev/null || true
  : > "$BUILD_LOG" || true
  log SUCCESS "Clean complete"
}

compile_asm() {
  local file="$1"
  local output="$2"
  log INFO "Assembling $file"
  nasm -f elf64 "$file" -o "$output" >>"$BUILD_LOG" 2>&1 || die "nasm failed for $file"
}

compile_c() {
  local file="$1"
  local output="$2"
  log INFO "Compiling $file"

  if [[ "${CC:-}" == "x86_64-elf-gcc" ]]; then
    "$CC" -m64 -c "$file" -o "$output" -Wall -Wextra -ffreestanding -fno-stack-protector -mcmodel=large -mno-red-zone >>"$BUILD_LOG" 2>&1 \
      || die "C compile failed for $file"
    return 0
  fi

  "$CC" -target "$TARGET_TRIPLE" -m64 -c "$file" -o "$output" \
    -Wall -Wextra -ffreestanding -fno-stack-protector -mcmodel=large -mno-red-zone \
    >>"$BUILD_LOG" 2>&1 || die "C compile failed for $file"
}

link_kernel() {
  [[ -f "link.ld" ]] || die "Missing linker script: link.ld"

  log INFO "Linking kernel"

  "$LD" -m elf_x86_64 -T link.ld -o "$KERNEL_OUT" "$@" >>"$BUILD_LOG" 2>&1 || die "Link failed"
  [[ -f "$KERNEL_OUT" ]] || die "Kernel output missing after link: $KERNEL_OUT"
  log SUCCESS "Kernel built: $KERNEL_OUT"
}

build_kernel() {
  mkdir -p "$OBJ_DIR"
  ensure_elf_toolchain

  compile_asm "core/kernel.asm" "$OBJ_DIR/kasm.o"
  compile_asm "core/idt.asm" "$OBJ_DIR/idt.o"
  compile_asm "core/context_switch.asm" "$OBJ_DIR/context_switch.o"

  compile_c "core/kernel.c" "$OBJ_DIR/kc.o"
  compile_c "core/console.c" "$OBJ_DIR/console.o"
  compile_c "core/utils.c" "$OBJ_DIR/utils.o"
  compile_c "core/pop_module.c" "$OBJ_DIR/pop_module.o"
  compile_c "pops/shimjapii_pop.c" "$OBJ_DIR/shimjapii_pop.o"
  compile_c "pops/spinner_pop.c" "$OBJ_DIR/spinner_pop.o"
  compile_c "pops/uptime_pop.c" "$OBJ_DIR/uptime_pop.o"
  compile_c "pops/halt_pop.c" "$OBJ_DIR/halt_pop.o"
  compile_c "pops/filesystem_pop.c" "$OBJ_DIR/filesystem_pop.o"
  compile_c "core/multiboot2.c" "$OBJ_DIR/multiboot2.o"
  compile_c "pops/sysinfo_pop.c" "$OBJ_DIR/sysinfo_pop.o"
  compile_c "pops/memory_pop.c" "$OBJ_DIR/memory_pop.o"
  compile_c "pops/cpu_pop.c" "$OBJ_DIR/cpu_pop.o"
  compile_c "pops/dolphin_pop.c" "$OBJ_DIR/dolphin_pop.o"
  compile_c "core/timer.c" "$OBJ_DIR/timer.o"
  compile_c "core/scheduler.c" "$OBJ_DIR/scheduler.o"
  compile_c "core/memory.c" "$OBJ_DIR/memory.o"
  compile_c "core/init.c" "$OBJ_DIR/init.o"
  compile_c "core/syscall.c" "$OBJ_DIR/syscall.o"

  local objs=(
    "$OBJ_DIR/kasm.o"
    "$OBJ_DIR/kc.o"
    "$OBJ_DIR/console.o"
    "$OBJ_DIR/utils.o"
    "$OBJ_DIR/pop_module.o"
    "$OBJ_DIR/shimjapii_pop.o"
    "$OBJ_DIR/idt.o"
    "$OBJ_DIR/context_switch.o"
    "$OBJ_DIR/spinner_pop.o"
    "$OBJ_DIR/uptime_pop.o"
    "$OBJ_DIR/halt_pop.o"
    "$OBJ_DIR/filesystem_pop.o"
    "$OBJ_DIR/multiboot2.o"
    "$OBJ_DIR/sysinfo_pop.o"
    "$OBJ_DIR/memory_pop.o"
    "$OBJ_DIR/cpu_pop.o"
    "$OBJ_DIR/dolphin_pop.o"
    "$OBJ_DIR/timer.o"
    "$OBJ_DIR/scheduler.o"
    "$OBJ_DIR/memory.o"
    "$OBJ_DIR/init.o"
    "$OBJ_DIR/syscall.o"
  )

  for obj in "${objs[@]}"; do
    [[ -f "$obj" ]] || die "Missing object file: $obj"
  done

  link_kernel "${objs[@]}"
}

find_grub_mkrescue() {
  if have grub2-mkrescue; then
    printf '%s' "grub2-mkrescue"
    return 0
  fi
  if have grub-mkrescue; then
    printf '%s' "grub-mkrescue"
    return 0
  fi
  if have i686-elf-grub-mkrescue; then
    printf '%s' "i686-elf-grub-mkrescue"
    return 0
  fi
  if have x86_64-elf-grub-mkrescue; then
    printf '%s' "x86_64-elf-grub-mkrescue"
    return 0
  fi
  return 1
}

create_iso() {
  [[ -f "$KERNEL_OUT" ]] || die "Kernel not found ($KERNEL_OUT). Build the kernel first."

  local grub_mkrescue=""
  grub_mkrescue="$(find_grub_mkrescue || true)"
  if [[ -z "$grub_mkrescue" && "$HOST_OS" == "darwin" ]]; then
    log WARNING "grub-mkrescue not found. Trying to install ISO tooling via Homebrew..."
    have brew || die "Homebrew required to install ISO tooling on macOS."
    brew_install_if_missing xorriso
    brew_install_if_missing mtools
    # Install both; prefer i686-elf-grub-mkrescue for BIOS boot.
    brew_install_if_missing i686-elf-grub
    brew_install_if_missing x86_64-elf-grub
    grub_mkrescue="$(find_grub_mkrescue || true)"
  fi

  [[ -n "$grub_mkrescue" ]] || die "grub-mkrescue not found. Install GRUB mkrescue tooling, then re-run."

  log INFO "Creating ISO via $grub_mkrescue"
  rm -rf isodir
  mkdir -p isodir/boot/grub
  cp "$KERNEL_OUT" isodir/boot/kernel

  cat > isodir/boot/grub/grub.cfg <<'EOF'
set timeout=3
set default=0

menuentry "Popcorn Kernel x64" {
    echo "Loading Popcorn kernel..."
    multiboot2 /boot/kernel
    echo "Booting kernel..."
    boot
}
EOF

  "$grub_mkrescue" -o "$ISO_OUT" isodir >>"$BUILD_LOG" 2>&1 || die "ISO creation failed (grub-mkrescue)."
  rm -rf isodir

  [[ -f "$ISO_OUT" ]] || die "ISO output missing: $ISO_OUT"
  log SUCCESS "ISO created: $ISO_OUT"
}

run_qemu() {
  [[ -f "$ISO_OUT" ]] || create_iso
  log INFO "Starting QEMU: RAM=${QEMU_MEMORY}MB cores=${QEMU_CORES}"
  qemu-system-x86_64 -cdrom "$ISO_OUT" -cpu qemu64 -m "$QEMU_MEMORY" -smp "$QEMU_CORES" -serial stdio
  log SUCCESS "QEMU session ended"
}

show_logs() {
  if have dialog; then
    dialog --title "Build Logs" --textbox "$BUILD_LOG" 22 80 || true
  else
    printf '%s\n' "Logs: $BUILD_LOG"
    printf '%s\n' "----"
    tail -n 200 "$BUILD_LOG" 2>/dev/null || true
  fi
}

settings_menu() {
  if ! have dialog; then
    log WARNING "dialog not installed; settings menu unavailable."
    return 0
  fi

  while true; do
    local choice=""
    choice="$(dialog --title "Settings" \
      --menu "Configure build options:" 15 60 6 \
      1 "Set QEMU Memory (Current: ${QEMU_MEMORY}MB)" \
      2 "Set QEMU Cores (Current: ${QEMU_CORES})" \
      3 "Save Settings" \
      4 "Back" \
      2>&1 >/dev/tty)" || true

    case "$choice" in
      1)
        local mem=""
        mem="$(dialog --inputbox "Enter memory in MB:" 8 40 "$QEMU_MEMORY" 2>&1 >/dev/tty)" || true
        [[ "$mem" =~ ^[0-9]+$ ]] && QEMU_MEMORY="$mem"
        ;;
      2)
        local cores=""
        cores="$(dialog --inputbox "Enter number of cores:" 8 40 "$QEMU_CORES" 2>&1 >/dev/tty)" || true
        [[ "$cores" =~ ^[0-9]+$ ]] && QEMU_CORES="$cores"
        ;;
      3)
        save_config
        dialog --title "Success" --msgbox "Settings saved." 7 40 || true
        ;;
      4|"")
        break
        ;;
    esac
  done
}

main_menu() {
  if ! have dialog; then
    cat <<'EOF'
Popcorn build (no dialog UI available)

Usage:
  ./src/build.sh build        Build kernel
  ./src/build.sh iso          Create ISO (builds kernel if missing)
  ./src/build.sh run          Run in QEMU (creates ISO if missing)
  ./src/build.sh clean        Remove artifacts
  ./src/build.sh logs         Show last logs
EOF
    return 0
  fi

  while true; do
    local choice=""
    choice="$(dialog --title "Popcorn Build System (macOS-friendly)" \
      --menu "Choose an operation:" 18 70 9 \
      1 "Build Kernel" \
      2 "Build Kernel + Create ISO" \
      3 "Create ISO (kernel must exist)" \
      4 "Run in QEMU (auto-creates ISO)" \
      5 "Clean Build Files" \
      6 "Show Build Logs" \
      7 "Settings" \
      8 "Exit" \
      2>&1 >/dev/tty)" || true

    case "$choice" in
      1) clean_build; build_kernel; dialog --title "Success" --msgbox "Kernel built: $KERNEL_OUT" 8 50 || true ;;
      2) clean_build; build_kernel; create_iso; dialog --title "Success" --msgbox "Built: $KERNEL_OUT\nISO: $ISO_OUT" 9 50 || true ;;
      3) create_iso; dialog --title "Success" --msgbox "ISO created: $ISO_OUT" 8 50 || true ;;
      4) clear; run_qemu ;;
      5) clean_build; dialog --title "Success" --msgbox "Clean complete." 7 40 || true ;;
      6) show_logs ;;
      7) settings_menu ;;
      8|"") clear; exit 0 ;;
    esac
  done
}

usage() {
  cat <<'EOF'
Usage: ./src/build.sh [command]

Commands:
  build     Build kernel
  iso       Create ISO (builds kernel if missing)
  run       Run in QEMU (creates ISO if missing)
  clean     Remove build artifacts
  logs      Show build logs
  menu      Launch dialog UI (if installed)
EOF
}

main() {
  check_dependencies
  load_config

  local cmd="${1:-menu}"
  case "$cmd" in
    build) build_kernel ;;
    iso) [[ -f "$KERNEL_OUT" ]] || build_kernel; create_iso ;;
    run) [[ -f "$KERNEL_OUT" ]] || build_kernel; [[ -f "$ISO_OUT" ]] || create_iso; run_qemu ;;
    clean) clean_build ;;
    logs) show_logs ;;
    menu) main_menu ;;
    -h|--help|help) usage ;;
    *) usage; die "Unknown command: $cmd" ;;
  esac
}

main "$@"