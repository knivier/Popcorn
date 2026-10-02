[[ -n "${POPCORN_BUILD_IMG_UEFI:-}" ]] && return 0
POPCORN_BUILD_IMG_UEFI=1

: "${POPCORN_TARGET:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
# shellcheck source=kernel.sh
source "$(dirname "${BASH_SOURCE[0]}")/kernel.sh"
# shellcheck source=uefi.sh
source "$(dirname "${BASH_SOURCE[0]}")/uefi.sh"

IMG_OUT="${IMG_OUT:-$POPCORN_TARGET/popcorn-uefi.img}"
IMG_SIZE_MB="${IMG_SIZE_MB:-64}"

build_uefi_img() {
  [[ -f "$KERNEL_OUT" ]] || die "Build kernel first: ./scripts/core.sh build"
  [[ -f "$UEFI_OUT" ]] || die "Build UEFI loader first: ./scripts/core.sh uefi"

  local stage
  stage="$(mktemp -d)"
  uefi_stage_layout "$stage"
  populate_fat_image "$IMG_OUT" "$stage" "$IMG_SIZE_MB"
  rm -rf "$stage"

  log SUCCESS "UEFI disk image: $IMG_OUT"
  log INFO "Flash with Balena Etcher (use the .img file, not the .iso)."
}
