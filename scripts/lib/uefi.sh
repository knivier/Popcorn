[[ -n "${POPCORN_BUILD_UEFI:-}" ]] && return 0
POPCORN_BUILD_UEFI=1

: "${POPCORN_SRC:?}"
: "${POPCORN_TARGET:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

UEFI_OBJ="${UEFI_OBJ:-$OBJ_DIR/bootx64_efi.o}"
UEFI_OUT="${UEFI_OUT:-$POPCORN_TARGET/BOOTX64.EFI}"
UEFI_STAGING_DIR="${UEFI_STAGING_DIR:-$POPCORN_TARGET/uefi_usb/EFI/BOOT}"

build_uefi() {
  have clang || die "clang is required for UEFI loader"
  have lld-link || die "lld-link is required (install llvm/lld)"

  mkdir -p "$OBJ_DIR" "$UEFI_STAGING_DIR"

  clang -target x86_64-unknown-windows \
    -ffreestanding -fshort-wchar -fno-stack-protector -mno-red-zone \
    -Wall -Wextra -c "$POPCORN_SRC/uefi/bootx64.c" -o "$UEFI_OBJ"

  lld-link /subsystem:efi_application /entry:efi_main /nodefaultlib /machine:x64 \
    /out:"$UEFI_OUT" "$UEFI_OBJ"

  cp "$UEFI_OUT" "$UEFI_STAGING_DIR/BOOTX64.EFI"
  log SUCCESS "UEFI loader: $UEFI_OUT"
  log INFO "USB staging: $UEFI_STAGING_DIR/BOOTX64.EFI"
}
