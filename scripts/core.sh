#!/usr/bin/env bash
# Popcorn unified build entry point. Outputs land in target/.
set -Eeuo pipefail

POPCORN_SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
POPCORN_ROOT="$(cd "$POPCORN_SCRIPTS/.." && pwd)"
POPCORN_SRC="$POPCORN_ROOT/src"
POPCORN_TARGET="$POPCORN_ROOT/target"
export POPCORN_SCRIPTS POPCORN_ROOT POPCORN_SRC POPCORN_TARGET

mkdir -p "$POPCORN_TARGET"
cd "$POPCORN_ROOT"

# shellcheck source=lib/common.sh
source "$POPCORN_SCRIPTS/lib/common.sh"
# shellcheck source=lib/kernel.sh
source "$POPCORN_SCRIPTS/lib/kernel.sh"
# shellcheck source=lib/uefi.sh
source "$POPCORN_SCRIPTS/lib/uefi.sh"
# shellcheck source=lib/img-uefi.sh
source "$POPCORN_SCRIPTS/lib/img-uefi.sh"
# shellcheck source=lib/iso-uefi.sh
source "$POPCORN_SCRIPTS/lib/iso-uefi.sh"
# shellcheck source=lib/qemu-uefi.sh
source "$POPCORN_SCRIPTS/lib/qemu-uefi.sh"

usage() {
  cat <<'EOF'
Popcorn build system — ./scripts/core.sh <command>

Build:
  build       Compile kernel into target/kernel
  uefi        Build target/BOOTX64.EFI
  img         Build target/popcorn-uefi.img (kernel + UEFI)
  iso         Build legacy target/popcorn.iso (GRUB + Multiboot2)
  iso-uefi    Build target/popcorn-uefi.iso
  all         build + uefi + img

Run:
  run         QEMU with legacy ISO
  run-uefi    QEMU with popcorn-uefi.img (interactive)

Test:
  test-uefi   QEMU smoke: UEFI img + GRUB ISO + #PF dump
  test-install QEMU-only auto-install (never flash the test img; rebuilds a safe img after)
  test-pf     Rebuild with POPCORN_TEST_PF; expect #PF COM1 dump

Other:
  clean       Remove target/
  logs        Show build log
  help        This message

Wrappers: ./scripts/macos.sh ./scripts/fedora.sh ./scripts/linux.sh
Windows:  powershell -File scripts/win.ps1 all|run-uefi  (WSL, safe img only)
GUIs:      ./scripts/gui-macos.py ./scripts/gui-fedora.py
           ./scripts/gui-linux.py ./scripts/gui-win.py
EOF
}

build_all_uefi() {
  build_kernel
  build_uefi
  build_uefi_img
}

# True if QEMU would boot a stale kernel (missing artifacts, or src newer than target/kernel).
uefi_artifacts_stale() {
  local img="${UEFI_IMG:-$POPCORN_TARGET/popcorn-uefi.img}"
  local efi="${UEFI_OUT:-$POPCORN_TARGET/BOOTX64.EFI}"
  [[ -f "$KERNEL_OUT" && -f "$efi" && -f "$img" ]] || return 0
  find "$POPCORN_SRC" \
    \( -name '*.c' -o -name '*.h' -o -name '*.asm' -o -name '*.rs' -o -name '*.toml' -o -name '*.ld' \) \
    -newer "$KERNEL_OUT" -print -quit 2>/dev/null | grep -q .
}

main() {
  local cmd="${1:-help}"
  shift || true

  load_config

  case "$cmd" in
    build)
      check_kernel_dependencies
      build_kernel
      ;;
    uefi)
      build_uefi
      ;;
    img)
      check_kernel_dependencies
      build_kernel
      build_uefi
      build_uefi_img
      ;;
    iso)
      check_kernel_dependencies
      [[ -f "$KERNEL_OUT" ]] || build_kernel
      create_legacy_iso
      ;;
    iso-uefi)
      check_kernel_dependencies
      build_kernel
      build_uefi
      build_uefi_iso
      ;;
    all)
      check_kernel_dependencies
      build_all_uefi
      ;;
    run)
      check_kernel_dependencies
      [[ -f "$KERNEL_OUT" ]] || build_kernel
      [[ -f "$ISO_OUT" ]] || create_legacy_iso
      run_legacy_qemu
      ;;
    run-uefi)
      check_kernel_dependencies
      if uefi_artifacts_stale; then
        log INFO "Rebuilding UEFI image so source edits are in the guest"
        build_all_uefi
      else
        log INFO "Using existing artifacts in $POPCORN_TARGET (run './scripts/core.sh all' to force rebuild)"
      fi
      assert_kernel_not_auto_install
      qemu_uefi_run_interactive
      ;;
    test-uefi)
      check_kernel_dependencies
      build_all_uefi
      qemu_uefi_smoke
      ;;
    test-pf)
      check_kernel_dependencies
      qemu_uefi_test_pf
      ;;
    test-uefi-stability)
      check_kernel_dependencies
      build_all_uefi
      qemu_uefi_test_stability
      ;;
    test-install)
      check_kernel_dependencies
      log WARN "Auto-install kernel is QEMU-only. NEVER flash target/UNSAFE/*."
      export POPCORN_CFLAGS="${POPCORN_CFLAGS:-} -DPOPCORN_TEST_INSTALL"
      mkdir -p "$POPCORN_TARGET/UNSAFE"
      IMG_OUT="$POPCORN_TARGET/UNSAFE/UNSAFE-popcorn-uefi-test-install.img"
      UEFI_IMG="$IMG_OUT"
      restore_safe_img() {
        unset POPCORN_CFLAGS
        IMG_OUT="$POPCORN_TARGET/popcorn-uefi.img"
        UEFI_IMG="$IMG_OUT"
        log INFO "Rebuilding hardware-safe popcorn-uefi.img (no auto-install)"
        build_all_uefi
      }
      trap restore_safe_img EXIT
      rm -f "${USB_DATA_IMG:-$POPCORN_TARGET/UNSAFE/UNSAFE-qemu-usb-data.img}"
      build_all_uefi
      qemu_uefi_test_install
      ;;
    clean)
      clean_build_artifacts
      ;;
    logs)
      show_logs
      ;;
    help|--help|-h)
      usage
      ;;
    *)
      die "Unknown command: $cmd (try: ./scripts/core.sh help)"
      ;;
  esac
}

main "$@"
