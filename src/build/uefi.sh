#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

OBJ_DIR="${OBJ_DIR:-obj}"
UEFI_OBJ="$OBJ_DIR/bootx64_efi.o"
UEFI_OUT="${UEFI_OUT:-BOOTX64.EFI}"
UEFI_STAGING_DIR="${UEFI_STAGING_DIR:-uefi_usb/EFI/BOOT}"

have() { command -v "$1" >/dev/null 2>&1; }

if ! have clang; then
  echo "error: clang is required" >&2
  exit 1
fi
if ! have lld-link; then
  echo "error: lld-link is required (brew install llvm)" >&2
  exit 1
fi

mkdir -p "$OBJ_DIR" "$UEFI_STAGING_DIR"

clang -target x86_64-unknown-windows \
  -ffreestanding -fshort-wchar -fno-stack-protector -mno-red-zone \
  -Wall -Wextra -c uefi/bootx64.c -o "$UEFI_OBJ"

lld-link /subsystem:efi_application /entry:efi_main /nodefaultlib /machine:x64 \
  /out:"$UEFI_OUT" "$UEFI_OBJ"

cp "$UEFI_OUT" "$UEFI_STAGING_DIR/BOOTX64.EFI"

echo "Built: $SRC_DIR/$UEFI_OUT"
echo "USB staging: $SRC_DIR/$UEFI_STAGING_DIR/BOOTX64.EFI"
