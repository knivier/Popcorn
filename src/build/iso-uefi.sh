#!/usr/bin/env bash
# Build popcorn-uefi.iso: native BOOTX64.EFI + /boot/kernel (no GRUB on UEFI path).
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

ISO_OUT="${ISO_OUT:-popcorn-uefi.iso}"
ISODIR="${ISODIR:-isodir-uefi}"

have() { command -v "$1" >/dev/null 2>&1; }

bash build/macos.sh build
bash build/uefi.sh
bash build/make-efi-fat.sh

rm -rf "$ISODIR"
mkdir -p "$ISODIR/EFI/BOOT" "$ISODIR/boot"
cp "$SRC_DIR/BOOTX64.EFI" "$ISODIR/EFI/BOOT/BOOTX64.EFI"
cp "$SRC_DIR/kernel" "$ISODIR/boot/kernel"
cp "$SRC_DIR/kernel" "$ISODIR/EFI/BOOT/kernel"

if have xorriso; then
  xorriso -as mkisofs \
    -R -J -joliet-long \
    -o "$ISO_OUT" \
    --efi-boot EFI/BOOT/BOOTX64.EFI \
    -efi-boot-part "$SRC_DIR/efi_part.img" \
    --protective-msdos-label \
    -isohybrid-gpt-basdat \
    "$ISODIR"
elif have x86_64-elf-grub-mkrescue; then
  x86_64-elf-grub-mkrescue -o "$ISO_OUT" "$ISODIR" -- -efi-boot-part
elif have i686-elf-grub-mkrescue; then
  i686-elf-grub-mkrescue -o "$ISO_OUT" "$ISODIR" -- -efi-boot-part
else
  echo "error: need xorriso or grub-mkrescue for UEFI ISO" >&2
  exit 1
fi

rm -rf "$ISODIR"
echo "Created $SRC_DIR/$ISO_OUT (UEFI: BOOTX64.EFI -> /boot/kernel)"
