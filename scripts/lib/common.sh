# Shared Popcorn build helpers (sourced, not executed).
[[ -n "${POPCORN_BUILD_COMMON:-}" ]] && return 0
POPCORN_BUILD_COMMON=1

: "${POPCORN_ROOT:?POPCORN_ROOT must be set before sourcing lib/common.sh}"
: "${POPCORN_SRC:?POPCORN_SRC must be set before sourcing lib/common.sh}"
: "${POPCORN_TARGET:?POPCORN_TARGET must be set before sourcing lib/common.sh}"
POPCORN_SCRIPTS="${POPCORN_SCRIPTS:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

OBJ_DIR="${OBJ_DIR:-$POPCORN_TARGET/obj}"
BUILD_BASE="${BUILD_BASE:-$POPCORN_TARGET}"
BUILD_LOG="${BUILD_LOG:-$POPCORN_TARGET/build.log}"
CONFIG_FILE="${CONFIG_FILE:-$POPCORN_TARGET/.build_config}"
# Unix domain sockets fail on /mnt/c; keep runtime sockets on a native Linux fs.
RUNTIME_DIR="${RUNTIME_DIR:-/tmp/popcorn-qemu}"
HOST_OS="${HOST_OS:-$(uname -s | tr '[:upper:]' '[:lower:]')}"

GREEN=$'\033[0;32m'
RED=$'\033[0;31m'
YELLOW=$'\033[1;33m'
BLUE=$'\033[0;34m'
NC=$'\033[0m'

mkdir -p "$POPCORN_TARGET" "$OBJ_DIR" "$RUNTIME_DIR"

have() { command -v "$1" >/dev/null 2>&1; }

log() {
  local level="$1"
  shift
  local message="$*"
  local timestamp
  timestamp="$(date '+%Y-%m-%d %H:%M:%S')"
  printf '[%s] [%s] %s\n' "$timestamp" "$level" "$message" >>"$BUILD_LOG" 2>/dev/null || true
  # Always print to stderr so $(...) command substitutions never swallow logs into argv.
  case "$level" in
    INFO) printf '%s%s%s\n' "$BLUE" "$message" "$NC" >&2 ;;
    SUCCESS) printf '%s%s%s\n' "$GREEN" "$message" "$NC" >&2 ;;
    WARN|WARNING) printf '%s%s%s\n' "$YELLOW" "$message" "$NC" >&2 ;;
    ERROR) printf '%s%s%s\n' "$RED" "$message" "$NC" >&2 ;;
    *) printf '%s\n' "$message" >&2 ;;
  esac
}

die() {
  log ERROR "$*"
  log INFO "See logs at: $BUILD_LOG"
  exit 1
}

load_config() {
  if [[ -f "$CONFIG_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$CONFIG_FILE"
  fi
}

save_config() {
  mkdir -p "$POPCORN_TARGET"
  {
    printf 'QEMU_MEMORY=%q\n' "${QEMU_MEMORY:-256}"
    printf 'QEMU_CORES=%q\n' "${QEMU_CORES:-1}"
  } >"$CONFIG_FILE"
}

find_edk_code() {
  local p
  for p in \
    /opt/homebrew/share/qemu/edk2-x86_64-code.fd \
    /opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd \
    /usr/share/qemu/edk2-x86_64-code.fd \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/edk2/x64/OVMF_CODE.fd \
    /usr/share/edk2/ovmf/OVMF_CODE_4M.fd \
    /usr/share/edk2/ovmf/OVMF_CODE.fd \
    /usr/share/OVMF/OVMF_CODE.fd; do
    if [[ -f "$p" ]]; then
      printf '%s' "$p"
      return 0
    fi
  done
  return 1
}

# OVMF vars template that pairs with a given CODE image (same firmware build).
# A mismatched vars size shifts the variable store away from where OVMF expects it.
ovmf_vars_template_for() {
  local code="$1" dir base candidate
  dir="$(dirname "$code")"
  base="$(basename "$code")"
  case "$base" in
    edk2-x86_64-code.fd) candidate="$dir/edk2-i386-vars.fd" ;;
    OVMF_CODE_4M.fd) candidate="$dir/OVMF_VARS_4M.fd" ;;
    OVMF_CODE.fd) candidate="$dir/OVMF_VARS.fd" ;;
    *) candidate="" ;;
  esac
  if [[ -n "$candidate" && -f "$candidate" ]]; then
    printf '%s' "$candidate"
    return 0
  fi
  return 1
}

ensure_ovmf_vars() {
  local vars="${1:-$POPCORN_TARGET/ovmf_vars.fd}"
  local code="" template=""
  mkdir -p "$(dirname "$vars")"
  code="$(find_edk_code || true)"
  if [[ -n "$code" ]]; then
    template="$(ovmf_vars_template_for "$code" || true)"
  fi
  if [[ -z "$template" ]]; then
    local t
    for t in \
      /usr/share/OVMF/OVMF_VARS_4M.fd \
      /usr/share/edk2/ovmf/OVMF_VARS_4M.fd \
      /usr/share/edk2/ovmf/OVMF_VARS.fd \
      /usr/share/OVMF/OVMF_VARS.fd \
      /usr/share/edk2/x64/OVMF_VARS.fd \
      /opt/homebrew/share/qemu/edk2-i386-vars.fd \
      /usr/local/share/qemu/edk2-i386-vars.fd \
      /usr/share/qemu/edk2-i386-vars.fd; do
      if [[ -f "$t" ]]; then
        template="$t"
        break
      fi
    done
  fi
  if [[ -n "$template" ]]; then
    # Re-seed when missing or when a stale store has the wrong size for this firmware.
    if [[ ! -f "$vars" ]] || [[ "$(wc -c <"$vars" | tr -d ' ')" != "$(wc -c <"$template" | tr -d ' ')" ]]; then
      cp "$template" "$vars"
    fi
    return 0
  fi
  if [[ ! -f "$vars" ]]; then
    dd if=/dev/zero of="$vars" bs=1M count=4 status=none
  fi
}

# QEMU guests started by these scripts write their PID here (-pidfile), so
# stop/liveness checks only ever touch Popcorn's own guest, never other VMs.
QEMU_PIDFILE="${QEMU_PIDFILE:-$RUNTIME_DIR/popcorn-qemu.pid}"

# Print the PID of the running Popcorn guest; fail if there is none.
qemu_guest_pid() {
  local pid=""
  [[ -f "$QEMU_PIDFILE" ]] || return 1
  pid="$(tr -dc '0-9' <"$QEMU_PIDFILE" 2>/dev/null || true)"
  [[ -n "$pid" ]] || return 1
  # A stale pidfile (guest SIGKILLed) may name a reused PID: only trust QEMU.
  case "$(ps -p "$pid" -o comm= 2>/dev/null || true)" in
    *qemu-system*) printf '%s' "$pid" ;;
    *) return 1 ;;
  esac
}

qemu_guest_alive() {
  qemu_guest_pid >/dev/null
}

# Stop the Popcorn guest (and wait for it to exit so its pidfile lock and VNC
# port are free for the next launch). Unrelated QEMU instances are left alone.
qemu_kill_guest() {
  local pid="" i
  if pid="$(qemu_guest_pid)"; then
    kill "$pid" 2>/dev/null || true
    for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.25
    done
    if kill -0 "$pid" 2>/dev/null; then
      kill -9 "$pid" 2>/dev/null || true
      sleep 0.5
    fi
  fi
  rm -f "$QEMU_PIDFILE"
  return 0
}

# Copy kernel + BOOTX64.EFI into a staging tree (ESP layout).
uefi_stage_layout() {
  local stage="$1"
  mkdir -p "$stage/EFI/BOOT" "$stage/boot"
  cp "$UEFI_OUT" "$stage/EFI/BOOT/BOOTX64.EFI"
  cp "$KERNEL_OUT" "$stage/boot/kernel"
  # test-install only: a safe kernel beside the auto-install one, which the
  # install selftest stages onto its target (it never copies an auto-install kernel).
  cp "${STAGE_EFI_KERNEL:-$KERNEL_OUT}" "$stage/EFI/BOOT/kernel"
  # test-install only: the normal loader beside the test one. The installer
  # copies this instead of the test loader, so no target ever gets the latter.
  if [[ -n "${STAGE_EFI_LOADER:-}" ]]; then
    cp "$STAGE_EFI_LOADER" "$stage/EFI/BOOT/SAFEX64.EFI"
  fi
  popcorn_log_file "$stage/POPCORN.LOG"
  if declare -F firmware_stage >/dev/null; then firmware_stage "$stage"; fi
}

# POPCORN.LOG: the kernel log file on the boot stick (src/rust/popcorn_kernel/src/klog).
# Created here at its full size so the kernel never allocates clusters or writes the
# FAT/directories: it only rewrites sectors inside this file. Layout must match
# klog/fatlog.rs: a 512-byte header sector whose first line carries "boot N" and
# "used N", then padding of 63 spaces + '\n' per 64 bytes.
POPCORN_LOG_BYTES=1048576
popcorn_log_file() {
  local out="$1"
  local hdr line pad n=$((POPCORN_LOG_BYTES - 512))
  # Pure bash (no awk on minimal hosts): one 64-byte blank line, doubled past n.
  printf -v line '%63s\n' ''
  pad="$line"
  while ((${#pad} < n)); do pad+="$pad"; done
  hdr="POPCORN.LOG  boot 0  used 0 of $((POPCORN_LOG_BYTES - 512)) bytes  dropped 0  uptime 0.000 s
No boot has written this file yet. Popcorn writes its kernel log here, on the USB
stick it booted from, while it runs; read it on another computer afterwards."
  {
    printf '%-511s\n' "$hdr"
    printf '%s' "${pad:0:n}"
  } >"$out"
  local got
  got=$(wc -c <"$out")
  [[ "$got" -eq "$POPCORN_LOG_BYTES" ]] || die "POPCORN.LOG is $got bytes, expected $POPCORN_LOG_BYTES"
}

# Write a FAT32 image from a staged directory tree.
populate_fat_image() {
  local image="$1"
  local stage="$2"
  local size_mb="${3:-32}"

  rm -f "$image"
  dd if=/dev/zero of="$image" bs=1M count="$size_mb" status=none

  if have mkfs.vfat && have mcopy; then
    mkfs.vfat -F 32 -n POPCORN "$image"
    mcopy -s -i "$image" "$stage"/* ::
  elif have mformat && have mcopy; then
    mformat -F -h 32 -t 64 -n 64 -c 1 -v POPCORN -i "$image" ::
    mcopy -s -i "$image" "$stage"/* ::
  elif have newfs_msdos && have hdiutil; then
    newfs_msdos -F 32 -S 512 -v POPCORN "$image"
    local mnt dev
    mnt="$(mktemp -d)"
    dev="$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$image" | awk 'NR==1{print $1}')"
    mount -t msdos "$dev" "$mnt"
    mkdir -p "$mnt/EFI/BOOT" "$mnt/boot"
    cp "$stage/EFI/BOOT/"* "$mnt/EFI/BOOT/"
    cp "$stage/boot/kernel" "$mnt/boot/kernel"
    [[ -f "$stage/POPCORN.LOG" ]] && cp "$stage/POPCORN.LOG" "$mnt/POPCORN.LOG"
    [[ -d "$stage/FW" ]] && cp -R "$stage/FW" "$mnt/FW"
    umount "$mnt"
    hdiutil detach "$dev" >/dev/null
    rmdir "$mnt"
  else
    die "Need mkfs.vfat+mcopy, mformat+mcopy, or macOS newfs_msdos+hdiutil for FAT images"
  fi
}
