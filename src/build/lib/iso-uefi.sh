[[ -n "${POPCORN_BUILD_ISO_UEFI:-}" ]] && return 0
POPCORN_BUILD_ISO_UEFI=1

: "${POPCORN_SRC:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

UEFI_ISO_OUT="${UEFI_ISO_OUT:-popcorn-uefi.iso}"
EFI_PART="${EFI_PART:-efi_part.img}"
EFI_PART_MB="${EFI_PART_MB:-32}"
ISODIR="${ISODIR:-isodir-uefi}"

build_uefi_esp_part() {
  [[ -f "$POPCORN_SRC/BOOTX64.EFI" ]] || die "Build UEFI loader first: ./build/core.sh uefi"
  [[ -f "$POPCORN_SRC/kernel" ]] || die "Build kernel first: ./build/core.sh build"

  local stage
  stage="$(mktemp -d)"
  uefi_stage_layout "$stage"
  populate_fat_image "$EFI_PART" "$stage" "$EFI_PART_MB"
  rm -rf "$stage"
  log SUCCESS "FAT ESP image: $POPCORN_SRC/$EFI_PART"
}

build_uefi_iso() {
  [[ -f "$POPCORN_SRC/BOOTX64.EFI" ]] || die "Build UEFI loader first: ./build/core.sh uefi"
  [[ -f "$POPCORN_SRC/kernel" ]] || die "Build kernel first: ./build/core.sh build"
  [[ -f "$EFI_PART" ]] || build_uefi_esp_part

  rm -rf "$ISODIR"
  mkdir -p "$ISODIR/EFI/BOOT" "$ISODIR/boot"
  cp "$POPCORN_SRC/BOOTX64.EFI" "$ISODIR/EFI/BOOT/BOOTX64.EFI"
  cp "$POPCORN_SRC/kernel" "$ISODIR/boot/kernel"
  cp "$POPCORN_SRC/kernel" "$ISODIR/EFI/BOOT/kernel"

  if have xorriso; then
    xorriso -as mkisofs \
      -R -J -joliet-long \
      -o "$UEFI_ISO_OUT" \
      --efi-boot EFI/BOOT/BOOTX64.EFI \
      -efi-boot-part "$POPCORN_SRC/$EFI_PART" \
      --protective-msdos-label \
      -isohybrid-gpt-basdat \
      "$ISODIR"
  elif have x86_64-elf-grub-mkrescue; then
    x86_64-elf-grub-mkrescue -o "$UEFI_ISO_OUT" "$ISODIR" -- -efi-boot-part
  elif have i686-elf-grub-mkrescue; then
    i686-elf-grub-mkrescue -o "$UEFI_ISO_OUT" "$ISODIR" -- -efi-boot-part
  else
    die "Need xorriso or grub-mkrescue for UEFI ISO"
  fi

  rm -rf "$ISODIR"
  log SUCCESS "UEFI ISO: $POPCORN_SRC/$UEFI_ISO_OUT"
}
