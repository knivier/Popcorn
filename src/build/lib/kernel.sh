[[ -n "${POPCORN_BUILD_KERNEL:-}" ]] && return 0
POPCORN_BUILD_KERNEL=1

: "${POPCORN_SRC:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

TARGET_TRIPLE="x86_64-unknown-elf"
KERNEL_OUT="${KERNEL_OUT:-kernel}"
ISO_OUT="${ISO_OUT:-popcorn.iso}"
QEMU_MEMORY="${QEMU_MEMORY:-256}"
QEMU_CORES="${QEMU_CORES:-1}"

brew_install_if_missing() {
  local formula="$1"
  if brew list --formula "$formula" >/dev/null 2>&1; then
    return 0
  fi
  log INFO "Installing $formula via Homebrew..."
  brew install "$formula" >>"$BUILD_LOG" 2>&1 || die "Homebrew failed installing $formula"
}

check_kernel_dependencies() {
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
    have brew || die "Homebrew is required on macOS."
    for dep in "${missing[@]}"; do
      case "$dep" in
        nasm) brew_install_if_missing nasm ;;
        qemu-system-x86_64) brew_install_if_missing qemu ;;
        *) die "Missing dependency: $dep" ;;
      esac
    done
  else
    die "Missing dependencies: ${missing[*]}"
  fi
}

find_grub_mkrescue() {
  if have grub2-mkrescue; then printf '%s' "grub2-mkrescue"; return 0; fi
  if have grub-mkrescue; then printf '%s' "grub-mkrescue"; return 0; fi
  if have i686-elf-grub-mkrescue; then printf '%s' "i686-elf-grub-mkrescue"; return 0; fi
  if have x86_64-elf-grub-mkrescue; then printf '%s' "x86_64-elf-grub-mkrescue"; return 0; fi
  return 1
}

ensure_elf_toolchain() {
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
      have brew || die "Need Homebrew for LLVM (ld.lld)."
      log WARNING "Installing LLVM (ld.lld) via Homebrew..."
      brew_install_if_missing llvm
      export PATH="$(brew --prefix llvm)/bin:$PATH"
      have ld.lld || die "ld.lld not found after installing llvm"
      export LD="ld.lld"
    else
      die "Need an ELF linker (ld.lld or x86_64-elf-ld)."
    fi
  fi
  log INFO "Using toolchain: $CC + $LD (target $TARGET_TRIPLE)"
}

compile_asm() {
  log INFO "Assembling $1"
  nasm -f elf64 "$1" -o "$2" >>"$BUILD_LOG" 2>&1 || die "nasm failed for $1"
}

compile_c() {
  log INFO "Compiling $1"
  if [[ "${CC:-}" == "x86_64-elf-gcc" ]]; then
    "$CC" -m64 -c "$1" -o "$2" -Wall -Wextra -ffreestanding -fno-stack-protector \
      -mcmodel=large -mno-red-zone >>"$BUILD_LOG" 2>&1 || die "C compile failed for $1"
    return 0
  fi
  "$CC" -target "$TARGET_TRIPLE" -m64 -c "$1" -o "$2" \
    -Wall -Wextra -ffreestanding -fno-stack-protector -mcmodel=large -mno-red-zone \
    >>"$BUILD_LOG" 2>&1 || die "C compile failed for $1"
}

link_kernel() {
  [[ -f "$POPCORN_SRC/link.ld" ]] || die "Missing linker script: link.ld"
  log INFO "Linking kernel"
  "$LD" -m elf_x86_64 -T "$POPCORN_SRC/link.ld" -o "$KERNEL_OUT" "$@" >>"$BUILD_LOG" 2>&1 \
    || die "Link failed"
  [[ -f "$KERNEL_OUT" ]] || die "Kernel output missing: $KERNEL_OUT"
  log SUCCESS "Kernel built: $POPCORN_SRC/$KERNEL_OUT"
}

build_kernel() {
  mkdir -p "$OBJ_DIR"
  : >"$BUILD_LOG" 2>/dev/null || true
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
  compile_c "core/uefi_input.c" "$OBJ_DIR/uefi_input.o"
  compile_c "pops/sysinfo_pop.c" "$OBJ_DIR/sysinfo_pop.o"
  compile_c "pops/memory_pop.c" "$OBJ_DIR/memory_pop.o"
  compile_c "pops/cpu_pop.c" "$OBJ_DIR/cpu_pop.o"
  compile_c "pops/dolphin_pop.c" "$OBJ_DIR/dolphin_pop.o"
  compile_c "core/timer.c" "$OBJ_DIR/timer.o"
  compile_c "core/scheduler.c" "$OBJ_DIR/scheduler.o"
  compile_c "core/memory.c" "$OBJ_DIR/memory.o"
  compile_c "core/vmm.c" "$OBJ_DIR/vmm.o"
  compile_c "core/init.c" "$OBJ_DIR/init.o"
  compile_c "core/syscall.c" "$OBJ_DIR/syscall.o"

  local objs=(
    "$OBJ_DIR/kasm.o" "$OBJ_DIR/kc.o" "$OBJ_DIR/console.o" "$OBJ_DIR/utils.o"
    "$OBJ_DIR/pop_module.o" "$OBJ_DIR/shimjapii_pop.o" "$OBJ_DIR/idt.o"
    "$OBJ_DIR/context_switch.o" "$OBJ_DIR/spinner_pop.o" "$OBJ_DIR/uptime_pop.o"
    "$OBJ_DIR/halt_pop.o" "$OBJ_DIR/filesystem_pop.o" "$OBJ_DIR/multiboot2.o"
    "$OBJ_DIR/uefi_input.o" "$OBJ_DIR/sysinfo_pop.o" "$OBJ_DIR/memory_pop.o"
    "$OBJ_DIR/cpu_pop.o" "$OBJ_DIR/dolphin_pop.o" "$OBJ_DIR/timer.o"
    "$OBJ_DIR/scheduler.o" "$OBJ_DIR/memory.o" "$OBJ_DIR/vmm.o"
    "$OBJ_DIR/init.o" "$OBJ_DIR/syscall.o"
  )
  for obj in "${objs[@]}"; do
    [[ -f "$obj" ]] || die "Missing object file: $obj"
  done
  link_kernel "${objs[@]}"
}

create_legacy_iso() {
  [[ -f "$KERNEL_OUT" ]] || die "Kernel not found ($KERNEL_OUT). Run: ./build/core.sh build"
  local grub_mkrescue
  grub_mkrescue="$(find_grub_mkrescue || true)"
  if [[ -z "$grub_mkrescue" && "$HOST_OS" == "darwin" ]]; then
    log WARNING "Installing ISO tooling via Homebrew..."
    have brew || die "Homebrew required on macOS."
    brew_install_if_missing xorriso
    brew_install_if_missing mtools
    brew_install_if_missing i686-elf-grub
    brew_install_if_missing x86_64-elf-grub
    grub_mkrescue="$(find_grub_mkrescue || true)"
  fi
  [[ -n "$grub_mkrescue" ]] || die "grub-mkrescue not found."

  log INFO "Creating legacy ISO via $grub_mkrescue"
  rm -rf isodir
  mkdir -p isodir/boot/grub
  cp "$KERNEL_OUT" isodir/boot/kernel
  cat > isodir/boot/grub/grub.cfg <<'EOF'
insmod all_video
insmod efi_gop
set gfxmode=1024x768x32
set gfxpayload=1024x768x32
set timeout=3
set default=0
menuentry "Popcorn Kernel x64" {
    multiboot2 /boot/kernel
    boot
}
EOF
  "$grub_mkrescue" -o "$ISO_OUT" isodir >>"$BUILD_LOG" 2>&1 || die "ISO creation failed"
  rm -rf isodir
  [[ -f "$ISO_OUT" ]] || die "ISO output missing: $ISO_OUT"
  log SUCCESS "ISO created: $POPCORN_SRC/$ISO_OUT"
}

run_legacy_qemu() {
  [[ -f "$ISO_OUT" ]] || create_legacy_iso
  log INFO "Starting QEMU (legacy ISO): RAM=${QEMU_MEMORY}MB cores=${QEMU_CORES}"
  qemu-system-x86_64 -cdrom "$ISO_OUT" -cpu qemu64 -m "$QEMU_MEMORY" -smp "$QEMU_CORES" -serial stdio
}

show_logs() {
  if have dialog; then
    dialog --title "Build Logs" --textbox "$BUILD_LOG" 22 80 || true
  else
    printf 'Logs: %s\n----\n' "$BUILD_LOG"
    tail -n 200 "$BUILD_LOG" 2>/dev/null || true
  fi
}

clean_build_artifacts() {
  log INFO "Cleaning build artifacts..."
  rm -rf "$OBJ_DIR" "$KERNEL_OUT" "$ISO_OUT" isodir isodir-uefi isodir-uefi-fat 2>/dev/null || true
  rm -f "$POPCORN_SRC/BOOTX64.EFI" \
    "$POPCORN_SRC/popcorn-uefi.img" \
    "$POPCORN_SRC/popcorn-uefi.iso" \
    "$POPCORN_SRC/efi_part.img" 2>/dev/null || true
  rm -rf "$POPCORN_SRC/uefi_usb" 2>/dev/null || true
  rm -f "$BUILD_LOG" 2>/dev/null || true
  : >"$BUILD_LOG" 2>/dev/null || true
  log SUCCESS "Clean complete"
}
