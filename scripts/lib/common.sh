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
    WARNING) printf '%s%s%s\n' "$YELLOW" "$message" "$NC" >&2 ;;
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

ensure_ovmf_vars() {
  local vars="${1:-$POPCORN_TARGET/ovmf_vars.fd}"
  mkdir -p "$(dirname "$vars")"
  if [[ ! -f "$vars" ]]; then
    local template=""
    for template in \
      /usr/share/OVMF/OVMF_VARS_4M.fd \
      /usr/share/edk2/ovmf/OVMF_VARS_4M.fd \
      /usr/share/edk2/ovmf/OVMF_VARS.fd \
      /usr/share/OVMF/OVMF_VARS.fd \
      /usr/share/edk2/x64/OVMF_VARS.fd; do
      if [[ -f "$template" ]]; then
        cp "$template" "$vars"
        return 0
      fi
    done
    dd if=/dev/zero of="$vars" bs=1M count=4 status=none
  fi
}

qemu_kill_all() {
  killall qemu-system-x86_64 2>/dev/null || true
}

# Copy kernel + BOOTX64.EFI into a staging tree (ESP layout).
uefi_stage_layout() {
  local stage="$1"
  mkdir -p "$stage/EFI/BOOT" "$stage/boot"
  cp "$UEFI_OUT" "$stage/EFI/BOOT/BOOTX64.EFI"
  cp "$KERNEL_OUT" "$stage/boot/kernel"
  cp "$KERNEL_OUT" "$stage/EFI/BOOT/kernel"
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
    umount "$mnt"
    hdiutil detach "$dev" >/dev/null
    rmdir "$mnt"
  else
    die "Need mkfs.vfat+mcopy, mformat+mcopy, or macOS newfs_msdos+hdiutil for FAT images"
  fi
}
