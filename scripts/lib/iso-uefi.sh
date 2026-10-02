[[ -n "${POPCORN_BUILD_ISO_UEFI:-}" ]] && return 0
POPCORN_BUILD_ISO_UEFI=1

: "${POPCORN_TARGET:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
# shellcheck source=kernel.sh
source "$(dirname "${BASH_SOURCE[0]}")/kernel.sh"
# shellcheck source=uefi.sh
source "$(dirname "${BASH_SOURCE[0]}")/uefi.sh"

UEFI_ISO_OUT="${UEFI_ISO_OUT:-$POPCORN_TARGET/popcorn-uefi.iso}"
EFI_PART="${EFI_PART:-$POPCORN_TARGET/efi_part.img}"
EFI_PART_MB="${EFI_PART_MB:-32}"
ISODIR="${ISODIR:-$POPCORN_TARGET/isodir-uefi}"

build_uefi_esp_part() {
  [[ -f "$UEFI_OUT" ]] || die "Build UEFI loader first: ./scripts/core.sh uefi"
  [[ -f "$KERNEL_OUT" ]] || die "Build kernel first: ./scripts/core.sh build"

  local stage
  stage="$(mktemp -d)"
  uefi_stage_layout "$stage"
  populate_fat_image "$EFI_PART" "$stage" "$EFI_PART_MB"
  rm -rf "$stage"
  log SUCCESS "FAT ESP image: $EFI_PART"
}

build_uefi_iso() {
  [[ -f "$UEFI_OUT" ]] || die "Build UEFI loader first: ./scripts/core.sh uefi"
  [[ -f "$KERNEL_OUT" ]] || die "Build kernel first: ./scripts/core.sh build"
  [[ -f "$EFI_PART" ]] || build_uefi_esp_part

  rm -rf "$ISODIR"
  mkdir -p "$ISODIR/EFI/BOOT" "$ISODIR/boot"
  cp "$UEFI_OUT" "$ISODIR/EFI/BOOT/BOOTX64.EFI"
  cp "$KERNEL_OUT" "$ISODIR/boot/kernel"
  cp "$KERNEL_OUT" "$ISODIR/EFI/BOOT/kernel"

  if have xorriso; then
    xorriso -as mkisofs \
      -R -J -joliet-long \
      -o "$UEFI_ISO_OUT" \
      --efi-boot EFI/BOOT/BOOTX64.EFI \
      -efi-boot-part "$EFI_PART" \
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
  log SUCCESS "UEFI ISO: $UEFI_ISO_OUT"
}
