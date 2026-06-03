#!/usr/bin/env bash
# popcorn-uefi.img — whole-disk FAT32 for Balena/Rufus/dd (most reliable on laptop UEFI).
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

IMG_OUT="${IMG_OUT:-popcorn-uefi.img}"
SIZE_MB="${SIZE_MB:-64}"

bash build/macos.sh build
bash build/uefi.sh

rm -f "$IMG_OUT"
dd if=/dev/zero of="$IMG_OUT" bs=1M count="$SIZE_MB" status=none

STAGE="$(mktemp -d)"
mkdir -p "$STAGE/EFI/BOOT" "$STAGE/boot"
cp "$SRC_DIR/BOOTX64.EFI" "$STAGE/EFI/BOOT/BOOTX64.EFI"
cp "$SRC_DIR/kernel" "$STAGE/boot/kernel"
cp "$SRC_DIR/kernel" "$STAGE/EFI/BOOT/kernel"

if command -v mkfs.vfat >/dev/null 2>&1; then
  mkfs.vfat -F 32 -n POPCORN "$IMG_OUT"
  mcopy -s -i "$IMG_OUT" "$STAGE"/* ::
elif command -v mformat >/dev/null 2>&1 && command -v mcopy >/dev/null 2>&1; then
  mformat -F -h 32 -t 64 -n 64 -c 1 -v POPCORN -i "$IMG_OUT" ::
  mcopy -s -i "$IMG_OUT" "$STAGE"/* ::
else
  newfs_msdos -F 32 -S 512 -v POPCORN "$IMG_OUT"
  MNT="$(mktemp -d)"
  DEV="$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$SRC_DIR/$IMG_OUT" | awk 'NR==1{print $1}')"
  mount -t msdos "$DEV" "$MNT"
  mkdir -p "$MNT/EFI/BOOT" "$MNT/boot"
  cp "$STAGE/EFI/BOOT/"* "$MNT/EFI/BOOT/"
  cp "$STAGE/boot/kernel" "$MNT/boot/kernel"
  umount "$MNT"
  hdiutil detach "$DEV" >/dev/null
  rmdir "$MNT"
fi

rm -rf "$STAGE"
echo "Created $SRC_DIR/$IMG_OUT"
echo "Flash with Balena Etcher (use the .img file, not the .iso)."
