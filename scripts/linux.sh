#!/usr/bin/env bash
# Generic Linux entry (Debian/Ubuntu/Arch/...): CLI → core.sh; bare run → dialog.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
POPCORN_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
POPCORN_SRC="$POPCORN_ROOT/src"
POPCORN_TARGET="$POPCORN_ROOT/target"
POPCORN_SCRIPTS="$SCRIPT_DIR"
export POPCORN_ROOT POPCORN_SRC POPCORN_TARGET POPCORN_SCRIPTS

_core_commands="build uefi img iso iso-uefi all run run-uefi test-uefi test-uefi-stability clean logs help --help -h"
for c in $_core_commands; do
  if [[ "${1:-}" == "$c" ]]; then
    exec "$SCRIPT_DIR/core.sh" "$@"
  fi
done

# shellcheck source=lib/common.sh
source "$SCRIPT_DIR/lib/common.sh"
# shellcheck source=lib/kernel.sh
source "$SCRIPT_DIR/lib/kernel.sh"

check_kernel_dependencies
load_config

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
      3) save_config; dialog --title "Success" --msgbox "Settings saved." 7 40 || true ;;
      4|"") break ;;
    esac
  done
}

main_menu() {
  if ! have dialog; then
    exec "$SCRIPT_DIR/core.sh" help
  fi

  while true; do
    local choice=""
    choice="$(dialog --title "Popcorn Build System (Linux)" \
      --menu "Choose an operation:" 20 72 11 \
      1 "Build Kernel" \
      2 "Build Kernel + Legacy ISO" \
      3 "Create Legacy ISO" \
      4 "UEFI: build all (img for USB)" \
      5 "UEFI: run QEMU test suite" \
      6 "Run Legacy ISO in QEMU" \
      7 "Clean Build Files" \
      8 "Show Build Logs" \
      9 "Settings" \
      10 "Exit" \
      2>&1 >/dev/tty)" || true

    case "$choice" in
      1) clean_build_artifacts; build_kernel
         dialog --title "Success" --msgbox "Kernel built: $KERNEL_OUT" 8 50 || true ;;
      2) clean_build_artifacts; build_kernel; create_legacy_iso
         dialog --title "Success" --msgbox "Built: $KERNEL_OUT\nISO: $ISO_OUT" 9 50 || true ;;
      3) create_legacy_iso
         dialog --title "Success" --msgbox "ISO created: $ISO_OUT" 8 50 || true ;;
      4) "$SCRIPT_DIR/core.sh" all
         dialog --title "Success" --msgbox "UEFI image: $POPCORN_TARGET/popcorn-uefi.img" 8 55 || true ;;
      5) "$SCRIPT_DIR/core.sh" test-uefi
         dialog --title "UEFI tests" --msgbox "See terminal output for PASS/FAIL." 8 50 || true ;;
      6) clear; run_legacy_qemu ;;
      7) clean_build_artifacts; dialog --title "Success" --msgbox "Clean complete." 7 40 || true ;;
      8) show_logs ;;
      9) settings_menu ;;
      10|"") clear; exit 0 ;;
    esac
  done
}

main_menu
