[[ -n "${POPCORN_BUILD_KERNEL:-}" ]] && return 0
POPCORN_BUILD_KERNEL=1

: "${POPCORN_SRC:?}"
: "${POPCORN_TARGET:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

TARGET_TRIPLE="x86_64-unknown-elf"
KERNEL_OUT="${KERNEL_OUT:-$POPCORN_TARGET/kernel}"
ISO_OUT="${ISO_OUT:-$POPCORN_TARGET/popcorn.iso}"
ISO_STAGING="${ISO_STAGING:-$POPCORN_TARGET/isodir}"
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

ensure_rust_toolchain() {
  if [[ -f "${HOME:-}/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
  fi
  have rustc && have cargo || die "Rust required (install rustup from https://rustup.rs). Then: rustup target add x86_64-unknown-none"
  if ! rustup target list --installed 2>/dev/null | grep -qx 'x86_64-unknown-none'; then
    log INFO "Adding Rust target x86_64-unknown-none"
    rustup target add x86_64-unknown-none >>"$BUILD_LOG" 2>&1 \
      || die "rustup target add x86_64-unknown-none failed"
  fi
}

build_rust_kernel() {
  ensure_rust_toolchain
  local rust_dir="$POPCORN_SRC/rust"
  local out_dir="$POPCORN_TARGET/rust"
  local archive member
  mkdir -p "$out_dir" "$OBJ_DIR"
  [[ -f "$rust_dir/Cargo.toml" ]] || die "Missing Rust workspace: $rust_dir/Cargo.toml"
  log INFO "Building Rust crate popcorn_kernel (x86_64-unknown-none)"
  (
    cd "$rust_dir"
    CARGO_TARGET_DIR="$out_dir" cargo build --release --target x86_64-unknown-none
  ) >>"$BUILD_LOG" 2>&1 || die "cargo build failed — see $BUILD_LOG"
  archive="$out_dir/x86_64-unknown-none/release/libpopcorn_kernel.a"
  [[ -f "$archive" ]] || die "Missing $archive"
  # Link only the crate .o — pulling prebuilt core/compiler_builtins into this
  # high-half image breaks R_X86_64_32S boot relocations. rust_init only needs C.
  member="$(ar t "$archive" | grep '^popcorn_kernel-' | head -n1 || true)"
  [[ -n "$member" ]] || die "No popcorn_kernel-*.o in $archive"
  RUST_KERNEL_OBJ="$OBJ_DIR/popcorn_kernel_rs.o"
  (cd "$OBJ_DIR" && ar x "$archive" "$member" && mv -f "$member" "$RUST_KERNEL_OBJ") \
    >>"$BUILD_LOG" 2>&1 || die "Failed to extract $member from Rust archive"
  [[ -f "$RUST_KERNEL_OBJ" ]] || die "Missing $RUST_KERNEL_OBJ"
  export RUST_KERNEL_OBJ
  log SUCCESS "Rust object: $RUST_KERNEL_OBJ (from $archive)"
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
    ensure_rust_toolchain
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
    ensure_rust_toolchain
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
  nasm -f elf64 "$POPCORN_SRC/$1" -o "$2" >>"$BUILD_LOG" 2>&1 || die "nasm failed for $1"
}

compile_c() {
  log INFO "Compiling $1"
  # shellcheck disable=SC2086
  if [[ "${CC:-}" == "x86_64-elf-gcc" ]]; then
    "$CC" -m64 -c "$POPCORN_SRC/$1" -o "$2" -Wall -Wextra -ffreestanding -fno-stack-protector \
      -mcmodel=large -mno-red-zone ${POPCORN_CFLAGS:-} >>"$BUILD_LOG" 2>&1 || die "C compile failed for $1"
    return 0
  fi
  "$CC" -target "$TARGET_TRIPLE" -m64 -c "$POPCORN_SRC/$1" -o "$2" \
    -Wall -Wextra -ffreestanding -fno-stack-protector -mcmodel=large -mno-red-zone \
    ${POPCORN_CFLAGS:-} >>"$BUILD_LOG" 2>&1 || die "C compile failed for $1"
}

link_kernel() {
  [[ -f "$POPCORN_SRC/link.ld" ]] || die "Missing linker script: link.ld"
  log INFO "Linking kernel"
  "$LD" -m elf_x86_64 -T "$POPCORN_SRC/link.ld" -o "$KERNEL_OUT" "$@" >>"$BUILD_LOG" 2>&1 \
    || die "Link failed"
  [[ -f "$KERNEL_OUT" ]] || die "Kernel output missing: $KERNEL_OUT"
  log SUCCESS "Kernel built: $KERNEL_OUT"
}

build_kernel() {
  mkdir -p "$OBJ_DIR"
  : >"$BUILD_LOG" 2>/dev/null || true
  ensure_elf_toolchain
  build_rust_kernel

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
  compile_c "core/exception.c" "$OBJ_DIR/exception.o"
  compile_c "core/irq.c" "$OBJ_DIR/irq.o"
  compile_c "core/device.c" "$OBJ_DIR/device.o"
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
    "$OBJ_DIR/scheduler.o" "$OBJ_DIR/exception.o" "$OBJ_DIR/irq.o" "$OBJ_DIR/device.o"
    "$OBJ_DIR/memory.o" "$OBJ_DIR/vmm.o"
    "$OBJ_DIR/init.o" "$OBJ_DIR/syscall.o"
  )
  for obj in "${objs[@]}"; do
    [[ -f "$obj" ]] || die "Missing object file: $obj"
  done
  [[ -n "${RUST_KERNEL_OBJ:-}" && -f "$RUST_KERNEL_OBJ" ]] || die "Rust object missing — build_rust_kernel failed"
  link_kernel "${objs[@]}" "$RUST_KERNEL_OBJ"
}

create_legacy_iso() {
  [[ -f "$KERNEL_OUT" ]] || die "Kernel not found ($KERNEL_OUT). Run: ./scripts/core.sh build"
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
  rm -rf "$ISO_STAGING"
  mkdir -p "$ISO_STAGING/boot/grub"
  cp "$KERNEL_OUT" "$ISO_STAGING/boot/kernel"
  cat > "$ISO_STAGING/boot/grub/grub.cfg" <<'EOF'
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
  "$grub_mkrescue" -o "$ISO_OUT" "$ISO_STAGING" >>"$BUILD_LOG" 2>&1 || die "ISO creation failed"
  rm -rf "$ISO_STAGING"
  [[ -f "$ISO_OUT" ]] || die "ISO output missing: $ISO_OUT"
  log SUCCESS "ISO created: $ISO_OUT"
}

run_legacy_qemu() {
  [[ -f "$ISO_OUT" ]] || create_legacy_iso
  log INFO "Starting QEMU (legacy ISO): RAM=${QEMU_MEMORY}MB cores=${QEMU_CORES}"
  qemu-system-x86_64 -cdrom "$ISO_OUT" -cpu qemu64 -m "$QEMU_MEMORY" -smp "$QEMU_CORES" -serial stdio
}

# GRUB Multiboot2 ISO smoke: reach kmain (debugcon 'M') under IRQ PIT path.
qemu_legacy_smoke() {
  local dbg serial
  [[ -f "$KERNEL_OUT" ]] || build_kernel
  create_legacy_iso
  dbg="$POPCORN_TARGET/legacy-smoke-debugcon.log"
  serial="$POPCORN_TARGET/legacy-smoke-serial.log"
  rm -f "$dbg" "$serial"
  echo "== GRUB ISO (Multiboot2) =="
  qemu_kill_all || true
  qemu-system-x86_64 \
    -cdrom "$ISO_OUT" -cpu qemu64 -m "${QEMU_MEMORY:-512}" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$serial" \
    -display none -no-reboot -no-shutdown \
    -daemonize

  local waited=0
  while [[ $waited -lt 45 ]]; do
    if [[ -f "$dbg" ]] && grep -q 'M' "$dbg" 2>/dev/null; then
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  qemu_kill_all || true

  local body
  body="$(cat "$dbg" 2>/dev/null || true)$(cat "$serial" 2>/dev/null || true)"
  echo "legacy debugcon/serial: ${body:0:80}"
  case "$body" in *M*) ;; *)
    echo "FAIL: GRUB ISO did not reach kmain (no debugcon M)"
    return 1
    ;;
  esac
  echo "PASS: GRUB ISO reached kmain"
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
  log INFO "Cleaning build artifacts under $POPCORN_TARGET ..."
  rm -rf "$POPCORN_TARGET"
  mkdir -p "$POPCORN_TARGET" "$OBJ_DIR" "$RUNTIME_DIR"
  : >"$BUILD_LOG" 2>/dev/null || true
  log SUCCESS "Clean complete"
}
