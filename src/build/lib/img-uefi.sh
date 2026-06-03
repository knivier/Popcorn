[[ -n "${POPCORN_BUILD_IMG_UEFI:-}" ]] && return 0
POPCORN_BUILD_IMG_UEFI=1

: "${POPCORN_SRC:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

IMG_OUT="${IMG_OUT:-popcorn-uefi.img}"
IMG_SIZE_MB="${IMG_SIZE_MB:-64}"

build_uefi_img() {
  [[ -f "$POPCORN_SRC/kernel" ]] || die "Build kernel first: ./build/core.sh build"
  [[ -f "$POPCORN_SRC/BOOTX64.EFI" ]] || die "Build UEFI loader first: ./build/core.sh uefi"

  local stage
  stage="$(mktemp -d)"
  uefi_stage_layout "$stage"
  populate_fat_image "$IMG_OUT" "$stage" "$IMG_SIZE_MB"
  rm -rf "$stage"

  log SUCCESS "UEFI disk image: $POPCORN_SRC/$IMG_OUT"
  log INFO "Flash with Balena Etcher (use the .img file, not the .iso)."
}
