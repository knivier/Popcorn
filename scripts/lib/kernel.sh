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
  # Nightly + rust-src: rebuild core/alloc with large code model (build-std).
  if [[ -f "${HOME:-}/.cargo/env" ]]; then
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
  fi
  have rustup || die "rustup required (https://rustup.rs) for Popcorn Rust (nightly + rust-src)"
  have rustc && have cargo || die "Rust required (install rustup from https://rustup.rs)"
  local rust_dir="$POPCORN_SRC/rust"
  if [[ -f "$rust_dir/rust-toolchain.toml" ]]; then
    (cd "$rust_dir" && rustup show active-toolchain) >>"$BUILD_LOG" 2>&1 || true
    if ! (cd "$rust_dir" && rustup component list --installed 2>/dev/null | grep -qx 'rust-src'); then
      log INFO "Adding rust-src for nightly build-std"
      (cd "$rust_dir" && rustup component add rust-src) >>"$BUILD_LOG" 2>&1 \
        || die "rustup component add rust-src failed"
    fi
    if ! (cd "$rust_dir" && rustup target list --installed 2>/dev/null | grep -qx 'x86_64-unknown-none'); then
      log INFO "Adding Rust target x86_64-unknown-none"
      (cd "$rust_dir" && rustup target add x86_64-unknown-none) >>"$BUILD_LOG" 2>&1 \
        || die "rustup target add x86_64-unknown-none failed"
    fi
  fi
}

build_rust_kernel() {
  ensure_rust_toolchain
  local rust_dir="$POPCORN_SRC/rust"
  local out_dir="$POPCORN_TARGET/rust"
  local archive
  mkdir -p "$out_dir" "$OBJ_DIR"
  [[ -f "$rust_dir/Cargo.toml" ]] || die "Missing Rust workspace: $rust_dir/Cargo.toml"
  log INFO "Building Rust crate popcorn_kernel (build-std, large code model)"
  (
    cd "$rust_dir"
    CARGO_TARGET_DIR="$out_dir" cargo build --release --target x86_64-unknown-none
  ) >>"$BUILD_LOG" 2>&1 || die "cargo build failed — see $BUILD_LOG"
  archive="$out_dir/x86_64-unknown-none/release/libpopcorn_kernel.a"
  [[ -f "$archive" ]] || die "Missing $archive"
  # Full archive OK: core/alloc/compiler_builtins rebuilt with code-model=large.
  RUST_KERNEL_ARCHIVE="$archive"
  export RUST_KERNEL_ARCHIVE
  log SUCCESS "Rust archive: $RUST_KERNEL_ARCHIVE"
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
  compile_c "core/idt.c" "$OBJ_DIR/idt_c.o"
  compile_c "core/kbd.c" "$OBJ_DIR/kbd.o"
  compile_c "core/shell.c" "$OBJ_DIR/shell.o"
  compile_c "core/console.c" "$OBJ_DIR/console.o"
  compile_c "core/utils.c" "$OBJ_DIR/utils.o"
  compile_c "core/pop_module.c" "$OBJ_DIR/pop_module.o"
  # shimjapii / spinner / uptime / FAT32 FS are Rust (libpopcorn_kernel.a)
  compile_c "core/multiboot2.c" "$OBJ_DIR/multiboot2.o"
  compile_c "core/uefi_input.c" "$OBJ_DIR/uefi_input.o"
  # sysinfo / memory / cpu pops + info drives are Rust
  compile_c "pops/dolphin_pop.c" "$OBJ_DIR/dolphin_pop.o"
  compile_c "core/timer.c" "$OBJ_DIR/timer.o"
  compile_c "core/scheduler.c" "$OBJ_DIR/scheduler.o"
  compile_c "core/exception.c" "$OBJ_DIR/exception.o"
  compile_c "core/irq.c" "$OBJ_DIR/irq.o"
  compile_c "core/device.c" "$OBJ_DIR/device.o"
  compile_c "core/phase2_selftest.c" "$OBJ_DIR/phase2_selftest.o"
  compile_c "core/memory.c" "$OBJ_DIR/memory.o"
  compile_c "core/vmm.c" "$OBJ_DIR/vmm.o"
  compile_c "core/init.c" "$OBJ_DIR/init.o"
  compile_c "core/syscall.c" "$OBJ_DIR/syscall.o"

  local objs=(
    "$OBJ_DIR/kasm.o" "$OBJ_DIR/kc.o" "$OBJ_DIR/idt_c.o" "$OBJ_DIR/kbd.o" "$OBJ_DIR/shell.o"
    "$OBJ_DIR/console.o" "$OBJ_DIR/utils.o"
    "$OBJ_DIR/pop_module.o" "$OBJ_DIR/idt.o"
    "$OBJ_DIR/context_switch.o"
    "$OBJ_DIR/multiboot2.o"
    "$OBJ_DIR/uefi_input.o"
    "$OBJ_DIR/dolphin_pop.o" "$OBJ_DIR/timer.o"
    "$OBJ_DIR/scheduler.o" "$OBJ_DIR/exception.o" "$OBJ_DIR/irq.o" "$OBJ_DIR/device.o"
    "$OBJ_DIR/phase2_selftest.o" "$OBJ_DIR/memory.o" "$OBJ_DIR/vmm.o"
    "$OBJ_DIR/init.o" "$OBJ_DIR/syscall.o"
  )
  for obj in "${objs[@]}"; do
    [[ -f "$obj" ]] || die "Missing object file: $obj"
  done
  [[ -n "${RUST_KERNEL_ARCHIVE:-}" && -f "$RUST_KERNEL_ARCHIVE" ]] || die "Rust archive missing — build_rust_kernel failed"
  link_kernel "${objs[@]}" "$RUST_KERNEL_ARCHIVE"
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
  case "$body" in *r*) ;; *) echo "FAIL: GRUB missing r (rust_init)"; return 1 ;; esac
  case "$body" in *a*) ;; *) echo "FAIL: GRUB missing a (Rust alloc)"; return 1 ;; esac
  case "$body" in *I*) ;; *) echo "FAIL: GRUB missing Phase2 I (ioctl)"; return 1 ;; esac
  case "$body" in *B*) ;; *) echo "FAIL: GRUB missing Phase2 B (wait-queue)"; return 1 ;; esac
  case "$body" in *S*) ;; *) echo "FAIL: GRUB missing Phase2 S (sleep)"; return 1 ;; esac
  case "$body" in *2*) ;; *) echo "FAIL: GRUB missing Phase2 2"; return 1 ;; esac
  echo "PASS: GRUB ISO reached kmain (+ Phase2)"
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
