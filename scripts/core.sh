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
  test-uefi   QEMU smoke: stability + alive + debugcon

Other:
  clean       Remove target/
  logs        Show build log
  help        This message

Wrappers: ./scripts/macos.sh ./scripts/fedora.sh ./scripts/linux.sh
GUIs:      ./scripts/gui-macos.py ./scripts/gui-fedora.py
           ./scripts/gui-linux.py ./scripts/gui-win.py
EOF
}

build_all_uefi() {
  build_kernel
  build_uefi
  build_uefi_img
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
      if [[ ! -f "$KERNEL_OUT" || ! -f "${UEFI_OUT:-$POPCORN_TARGET/BOOTX64.EFI}" || ! -f "${UEFI_IMG:-$POPCORN_TARGET/popcorn-uefi.img}" ]]; then
        build_all_uefi
      else
        log INFO "Using existing artifacts in $POPCORN_TARGET (run './scripts/core.sh all' to rebuild)"
      fi
      qemu_uefi_run_interactive
      ;;
    test-uefi)
      check_kernel_dependencies
      build_all_uefi
      qemu_uefi_smoke
      ;;
    test-uefi-stability)
      check_kernel_dependencies
      build_all_uefi
      qemu_uefi_test_stability
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
