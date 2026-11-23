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
  # UEFI firmware boots from this ESP (-efi-boot-part), so it must always carry
  # the kernel + loader just built; a reused image would boot stale code.
  have xorriso || die "xorriso is required for the UEFI ISO (install the xorriso package, e.g. apt/dnf install xorriso, brew install xorriso)"
  if [[ "${POPCORN_CFLAGS:-}" == *POPCORN_TEST_INSTALL* ]]; then
    die "REFUSING UEFI ISO with the auto-install kernel (POPCORN_TEST_INSTALL is QEMU-only)"
  fi
  assert_kernel_not_auto_install
  assert_loader_not_test_install
  build_uefi_esp_part

  rm -rf "$ISODIR"
  mkdir -p "$ISODIR/EFI/BOOT" "$ISODIR/boot"
  cp "$UEFI_OUT" "$ISODIR/EFI/BOOT/BOOTX64.EFI"
  cp "$KERNEL_OUT" "$ISODIR/boot/kernel"
  cp "$KERNEL_OUT" "$ISODIR/EFI/BOOT/kernel"

  # grub-mkrescue is not a fallback: it needs xorriso itself and would embed its
  # own GRUB EFI image instead of booting our BOOTX64.EFI.
  xorriso -as mkisofs \
    -R -J -joliet-long \
    -o "$UEFI_ISO_OUT" \
    --efi-boot EFI/BOOT/BOOTX64.EFI \
    -efi-boot-part "$EFI_PART" \
    --protective-msdos-label \
    -isohybrid-gpt-basdat \
    "$ISODIR"

  rm -rf "$ISODIR"
  log SUCCESS "UEFI ISO: $UEFI_ISO_OUT"
}
