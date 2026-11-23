[[ -n "${POPCORN_BUILD_UEFI:-}" ]] && return 0
POPCORN_BUILD_UEFI=1

: "${POPCORN_SRC:?}"
: "${POPCORN_TARGET:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

UEFI_OBJ="${UEFI_OBJ:-$OBJ_DIR/bootx64_efi.o}"
UEFI_OUT="${UEFI_OUT:-$POPCORN_TARGET/BOOTX64.EFI}"

# The loader alone cannot boot (it reads \boot\kernel from its own volume), so no
# loader-only "USB staging" tree is produced. Flash target/popcorn-uefi.img.
build_uefi() {
  have clang || die "clang is required for UEFI loader"
  have lld-link || die "lld-link is required (install llvm/lld)"

  mkdir -p "$OBJ_DIR"

  # test-install only: a loader that boots the auto-install kernel inside a VM.
  # It must be written under target/UNSAFE, never to the flashable BOOTX64.EFI.
  local defs=()
  if [[ "${POPCORN_CFLAGS:-}" == *POPCORN_TEST_INSTALL* ]]; then
    case "$UEFI_OUT" in
      */UNSAFE/*) ;;
      *) die "REFUSING to build the test-install loader into $UEFI_OUT (use target/UNSAFE/)" ;;
    esac
    defs+=(-DPOPCORN_TEST_INSTALL)
  fi

  mkdir -p "$(dirname "$UEFI_OUT")"
  clang -target x86_64-unknown-windows \
    -ffreestanding -fshort-wchar -fno-stack-protector -mno-red-zone \
    -Wall -Wextra ${defs[@]+"${defs[@]}"} -c "$POPCORN_SRC/uefi/bootx64.c" -o "$UEFI_OBJ"

  lld-link /subsystem:efi_application /entry:efi_main /nodefaultlib /machine:x64 \
    /out:"$UEFI_OUT" "$UEFI_OBJ"

  if [[ "${POPCORN_CFLAGS:-}" == *POPCORN_TEST_INSTALL* ]]; then
    log WARNING "UNSAFE test-install loader: $UEFI_OUT — QEMU only, never flash"
  else
    assert_loader_not_test_install
    log SUCCESS "UEFI loader: $UEFI_OUT"
  fi
}

# Die if $UEFI_OUT is the QEMU test-install loader (boots auto-install kernels).
assert_loader_not_test_install() {
  if [[ -f "$UEFI_OUT" ]] && grep -aq 'POPCORN-UNSAFE-TEST-INSTALL-LOADER' "$UEFI_OUT"; then
    die "REFUSING: $UEFI_OUT is the test-install loader. Rebuild without POPCORN_TEST_INSTALL."
  fi
}
