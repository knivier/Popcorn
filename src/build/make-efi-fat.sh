#!/usr/bin/env bash
# Build a FAT filesystem image (no partition table) for xorriso -efi-boot-part.
# Contains EFI/BOOT/BOOTX64.EFI and boot/kernel so USB/ISO UEFI boots find the kernel.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

EFI_PART="${EFI_PART:-efi_part.img}"
STAGE="${STAGE:-isodir-uefi-fat}"
SIZE_MB="${SIZE_MB:-32}"

have() { command -v "$1" >/dev/null 2>&1; }

[[ -f "$SRC_DIR/BOOTX64.EFI" ]] || { echo "error: run build/uefi.sh first" >&2; exit 1; }
[[ -f "$SRC_DIR/kernel" ]] || { echo "error: run build/macos.sh build first" >&2; exit 1; }

rm -rf "$STAGE"
mkdir -p "$STAGE/EFI/BOOT" "$STAGE/boot"
cp "$SRC_DIR/BOOTX64.EFI" "$STAGE/EFI/BOOT/BOOTX64.EFI"
cp "$SRC_DIR/kernel" "$STAGE/boot/kernel"
# Also beside BOOTX64.EFI for firmware that only expose the ESP directory.
cp "$SRC_DIR/kernel" "$STAGE/EFI/BOOT/kernel"

rm -f "$EFI_PART"
dd if=/dev/zero of="$EFI_PART" bs=1M count="$SIZE_MB" status=none

if have mkfs.vfat; then
  mkfs.vfat -F 32 -n POPCORN "$EFI_PART"
  if have mcopy; then
    mcopy -s -i "$EFI_PART" "$STAGE"/* ::
  else
    echo "error: mtools mcopy required with mkfs.vfat" >&2
    exit 1
  fi
elif have mformat && have mcopy; then
  mformat -F -h 32 -t 64 -n 64 -c 1 -v POPCORN -i "$EFI_PART" ::
  mcopy -s -i "$EFI_PART" "$STAGE"/* ::
else
  # macOS: format raw image as FAT32 and copy via hdiutil
  if ! have newfs_msdos; then
    echo "error: need mkfs.vfat+mcopy, mformat+mcopy, or newfs_msdos (macOS)" >&2
    exit 1
  fi
  newfs_msdos -F 32 -S 512 -v POPCORN "$EFI_PART"
  MNT="$(mktemp -d)"
  DEV="$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$SRC_DIR/$EFI_PART" | awk 'NR==1{print $1}')"
  mount -t msdos "$DEV" "$MNT"
  mkdir -p "$MNT/EFI/BOOT" "$MNT/boot"
  cp "$STAGE/EFI/BOOT/"* "$MNT/EFI/BOOT/"
  cp "$STAGE/boot/kernel" "$MNT/boot/kernel"
  umount "$MNT"
  hdiutil detach "$DEV" >/dev/null
  rmdir "$MNT"
fi

rm -rf "$STAGE"
echo "FAT ESP image: $SRC_DIR/$EFI_PART"
