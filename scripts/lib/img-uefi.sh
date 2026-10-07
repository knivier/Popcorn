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

  if [[ "${POPCORN_CFLAGS:-}" == *POPCORN_TEST_INSTALL* ]]; then
    case "$(basename "$IMG_OUT")" in
      UNSAFE-*) ;;
      *)
        die "REFUSING to write auto-install kernel into $(basename "$IMG_OUT") — use target/UNSAFE/UNSAFE-*.img"
        ;;
    esac
  else
    if grep -aE -q 'install-selftest|rust_test_disk_install' "$KERNEL_OUT"; then
      die "REFUSING img: kernel still contains auto-install selftest"
    fi
  fi

  local stage
  mkdir -p "$(dirname "$IMG_OUT")"
  stage="$(mktemp -d)"
  uefi_stage_layout "$stage"
  populate_fat_image "$IMG_OUT" "$stage" "$IMG_SIZE_MB"
  rm -rf "$stage"

  if [[ "${POPCORN_CFLAGS:-}" == *POPCORN_TEST_INSTALL* ]]; then
    log WARN "UNSAFE image (QEMU auto-install): $IMG_OUT — do not flash"
  else
    log SUCCESS "UEFI disk image: $IMG_OUT"
    log INFO "Flash ONLY this file with Balena Etcher (never target/UNSAFE/*)."
  fi
}
