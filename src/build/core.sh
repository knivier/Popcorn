#!/usr/bin/env bash
# Popcorn unified build entry point.
set -Eeuo pipefail

POPCORN_BUILD="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
POPCORN_SRC="$(cd "$POPCORN_BUILD/.." && pwd)"
cd "$POPCORN_SRC"

# shellcheck source=lib/common.sh
source "$POPCORN_BUILD/lib/common.sh"
# shellcheck source=lib/kernel.sh
source "$POPCORN_BUILD/lib/kernel.sh"
# shellcheck source=lib/uefi.sh
source "$POPCORN_BUILD/lib/uefi.sh"
# shellcheck source=lib/img-uefi.sh
source "$POPCORN_BUILD/lib/img-uefi.sh"
# shellcheck source=lib/iso-uefi.sh
source "$POPCORN_BUILD/lib/iso-uefi.sh"
# shellcheck source=lib/qemu-uefi.sh
source "$POPCORN_BUILD/lib/qemu-uefi.sh"

usage() {
  cat <<'EOF'
Popcorn build system — ./build/core.sh <command>

Build:
  build       Compile kernel (popcorn.iso path uses GRUB/Multiboot)
  uefi        Build BOOTX64.EFI native loader
  img         Build popcorn-uefi.img (kernel + UEFI, for USB flash)
  iso         Build legacy popcorn.iso (GRUB + Multiboot2)
  iso-uefi    Build popcorn-uefi.iso (native UEFI, optional)
  all         build + uefi + img  (recommended before flashing hardware)

Run:
  run         QEMU with legacy popcorn.iso
  run-uefi    QEMU with popcorn-uefi.img (interactive)

Test:
  test-uefi   QEMU smoke: 30s stability + alive + debugcon

Other:
  clean       Remove build artifacts
  logs        Show build log
  help        This message

Wrappers: ./build/macos.sh and ./build/linux.sh delegate here for CLI commands.
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
      build_all_uefi
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
      die "Unknown command: $cmd (try: ./build/core.sh help)"
      ;;
  esac
}

main "$@"
