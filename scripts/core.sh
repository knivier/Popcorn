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
# shellcheck source=lib/lint.sh
source "$POPCORN_SCRIPTS/lib/lint.sh"

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
  test-uefi-stability  QEMU UEFI boot-and-stay-alive check only (also part of test-uefi)
  test-install QEMU-only auto-install (never flash the test img; rebuilds a safe img after)
  test-pf     Rebuild with POPCORN_TEST_PF; expect #PF COM1 dump (safe img rebuilt after)
  test-shell [cases.json] [args]  Type shell test cases (default
              scripts/tests/shell_cases.json) via QEMU sendkey on UEFI + GRUB and
              check the COM1 mirror (UNSAFE test imgs; safe img rebuilt after).
              Extra args go to scripts/lib/shell_test.py (--mode, --only, --jobs, --quick)
  test-quick  test-shell --quick: about 1 min smoke tier (cases marked "quick")

Checks:
  lint        Grant lint: no raw hardware access in drivers/virtio, drivers/net, net/;
              no static mut in the Rust crate
  test-host   Host unit tests (src/rust_host_tests: virtqueue model, checksum)

Other:
  stop        Stop the QEMU guest started by these scripts (other VMs untouched)
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

# True if QEMU would boot a stale kernel: missing artifacts, a kernel built from
# older src/ or with other POPCORN_CFLAGS (kernel_stale), or an img older than it.
uefi_artifacts_stale() {
  local img="${UEFI_IMG:-$POPCORN_TARGET/popcorn-uefi.img}"
  local efi="${UEFI_OUT:-$POPCORN_TARGET/BOOTX64.EFI}"
  [[ -f "$efi" && -f "$img" ]] || return 0
  kernel_stale && return 0
  [[ "$KERNEL_OUT" -nt "$img" || "$efi" -nt "$img" ]] && return 0
  return 1
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
      if kernel_stale; then
        build_kernel
      fi
      assert_kernel_not_auto_install
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
      if kernel_stale; then
        log INFO "Rebuilding kernel so source edits are in the guest"
        build_kernel
      fi
      # run_legacy_qemu repackages the ISO when the kernel is newer than it.
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
    test-quick)
      main test-shell "$@" --quick
      ;;
    test-shell)
      # Interactive shell tests: types every case in a JSON file through QEMU
      # HMP sendkey and checks the COM1 mirror (kernel built with
      # -DPOPCORN_SERIAL_CONSOLE). Usage:
      #   ./scripts/core.sh test-shell [cases.json] [shell_test.py args...]
      # e.g. test-shell scripts/tests/shell_cases.json --mode uefi --only 'mem_|cpu_'
      # Test kernel/img/ISO live under target/UNSAFE/; the EXIT trap always
      # rebuilds the hardware-safe target/kernel + popcorn-uefi.img.
      check_kernel_dependencies
      have python3 || die "python3 required for test-shell"
      local cases="$POPCORN_SCRIPTS/tests/shell_cases.json"
      if [[ $# -gt 0 && "$1" != -* ]]; then
        cases="$1"
        shift
      fi
      [[ -f "$cases" ]] || die "test-shell: cases file not found: $cases"
      mkdir -p "$POPCORN_TARGET/UNSAFE"
      # Globals, not locals: the EXIT trap runs after main has returned.
      SAFE_UEFI_OUT="$UEFI_OUT"
      restore_safe_shell_img() {
        qemu_kill_guest
        # Parallel shell-test guests (scripts/lib/shell_test.py lanes).
        local lane_pid
        for lane_pid in "$RUNTIME_DIR"/lane*/popcorn-qemu.pid; do
          [[ -f "$lane_pid" ]] && QEMU_PIDFILE="$lane_pid" qemu_kill_guest
        done
        unset POPCORN_CFLAGS
        IMG_OUT="$POPCORN_TARGET/popcorn-uefi.img"
        UEFI_IMG="$IMG_OUT"
        UEFI_OUT="$SAFE_UEFI_OUT"
        log INFO "Rebuilding hardware-safe popcorn-uefi.img (no serial console)"
        build_all_uefi
      }
      trap restore_safe_shell_img EXIT
      export POPCORN_CFLAGS="${POPCORN_CFLAGS:-} -DPOPCORN_SERIAL_CONSOLE"
      IMG_OUT="$POPCORN_TARGET/UNSAFE/UNSAFE-popcorn-shelltest.img"
      UEFI_IMG="$IMG_OUT"
      build_all_uefi
      ISO_OUT="$POPCORN_TARGET/UNSAFE/UNSAFE-popcorn-shelltest.iso" create_legacy_iso
      # Each parallel guest gets its own scratch disks (target/UNSAFE/UNSAFE-laneN-*),
      # recreated as zeroed files by the boot helpers.
      local rc=0
      SHELL_IMG="$IMG_OUT" SHELL_ISO="$POPCORN_TARGET/UNSAFE/UNSAFE-popcorn-shelltest.iso" \
        RUNTIME_DIR="$RUNTIME_DIR" \
        python3 "$POPCORN_SCRIPTS/lib/shell_test.py" --cases "$cases" "$@" || rc=$?
      [[ $rc -eq 0 ]] || log ERROR "test-shell: failures (rc=$rc); see target/shell-test/report.txt"
      return "$rc"
      ;;
    _shell-boot-uefi)
      # Internal (scripts/lib/shell_test.py): boot the serial-console UEFI test image.
      UEFI_IMG="${SHELL_IMG:-$POPCORN_TARGET/UNSAFE/UNSAFE-popcorn-shelltest.img}"
      POPCORN_SHELL_TEST=1 qemu_uefi_shell_boot
      ;;
    _shell-boot-grub)
      # Internal (scripts/lib/shell_test.py): boot the serial-console GRUB ISO.
      qemu_legacy_shell_boot
      ;;
    _shell-kill)
      qemu_kill_guest
      ;;
    test-install)
      check_kernel_dependencies
      log WARNING "Auto-install kernel is QEMU-only. NEVER flash target/UNSAFE/*."
      mkdir -p "$POPCORN_TARGET/UNSAFE"
      # Global, not local: the EXIT trap runs after main has returned.
      SAFE_UEFI_OUT="$UEFI_OUT"
      restore_safe_img() {
        unset POPCORN_CFLAGS STAGE_EFI_KERNEL STAGE_EFI_LOADER
        IMG_OUT="$POPCORN_TARGET/popcorn-uefi.img"
        UEFI_IMG="$IMG_OUT"
        UEFI_OUT="$SAFE_UEFI_OUT"
        log INFO "Rebuilding hardware-safe popcorn-uefi.img (no auto-install)"
        build_all_uefi
      }
      trap restore_safe_img EXIT
      # Safe kernel first: it rides along as \EFI\BOOT\kernel and is what the
      # selftest installs (an auto-install kernel is never staged).
      build_kernel
      export STAGE_EFI_KERNEL="$POPCORN_TARGET/UNSAFE/safe-kernel"
      cp "$KERNEL_OUT" "$STAGE_EFI_KERNEL"
      # And the normal loader, which the selftest installs instead of the test one.
      build_uefi
      export STAGE_EFI_LOADER="$POPCORN_TARGET/UNSAFE/safe-BOOTX64.EFI"
      cp "$UEFI_OUT" "$STAGE_EFI_LOADER"
      export POPCORN_CFLAGS="${POPCORN_CFLAGS:-} -DPOPCORN_TEST_INSTALL"
      IMG_OUT="$POPCORN_TARGET/UNSAFE/UNSAFE-popcorn-uefi-test-install.img"
      UEFI_IMG="$IMG_OUT"
      # The test loader (boots the auto-install kernel, VM only) stays under UNSAFE/.
      UEFI_OUT="$POPCORN_TARGET/UNSAFE/UNSAFE-BOOTX64.EFI"
      rm -f "${USB_DATA_IMG:-$POPCORN_TARGET/UNSAFE/UNSAFE-qemu-usb-data.img}"
      build_all_uefi
      qemu_uefi_test_install
      ;;
    lint)
      lint_grants
      ;;
    test-host)
      have cargo || die "cargo required for test-host"
      ( cd "$POPCORN_SRC/rust_host_tests" && cargo test --release 2>&1 ) || die "host unit tests failed"
      ;;
    stop)
      qemu_kill_guest
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
