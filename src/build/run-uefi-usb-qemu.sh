#!/usr/bin/env bash
# Boot popcorn-uefi.img as a USB disk (not optical) in QEMU.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

IMG="${SRC_DIR}/popcorn-uefi.img"
VARS="${SRC_DIR}/buildbase/ovmf_vars.fd"
MON_SOCK="${SRC_DIR}/buildbase/qemu-uefi-usb-mon.sock"
SCREEN="${SRC_DIR}/buildbase/uefi-usb-screen.ppm"

find_edk_code() {
  for p in \
    /opt/homebrew/share/qemu/edk2-x86_64-code.fd \
    /opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd; do
    [[ -f "$p" ]] && printf '%s' "$p" && return 0
  done
  return 1
}

bash build/img-uefi.sh

CODE="$(find_edk_code || true)"
[[ -n "$CODE" ]] || { echo "error: edk2-x86_64-code.fd not found" >&2; exit 1; }
[[ -f "$IMG" ]] || { echo "error: missing $IMG" >&2; exit 1; }

mkdir -p buildbase
[[ -f "$VARS" ]] || dd if=/dev/zero of="$VARS" bs=1m count=4 status=none

rm -f "$MON_SOCK" "$SCREEN"
killall qemu-system-x86_64 2>/dev/null || true

qemu-system-x86_64 \
  -machine q35 \
  -m 1024 \
  -cpu max \
  -drive "if=pflash,format=raw,readonly=on,file=$CODE" \
  -drive "if=pflash,format=raw,file=$VARS" \
  -drive "if=none,id=usbstick,format=raw,file=$IMG" \
  -device qemu-xhci,id=xhci \
  -device usb-storage,bus=xhci.0,drive=usbstick \
  -monitor "unix:$MON_SOCK,server,nowait" \
  -display none \
  -no-reboot \
  -daemonize

sleep 20
printf 'screendump %s\n' "$SCREEN" | nc -U "$MON_SOCK" 2>/dev/null || true
sleep 1
killall qemu-system-x86_64 2>/dev/null || true

python3 - <<PY
import re, sys
p = "$SCREEN"
try:
    with open(p, "rb") as f:
        hdr = f.read(64)
except FileNotFoundError:
    print("FAIL: no screendump")
    sys.exit(1)
m = re.search(br"P6\s+(\d+)\s+(\d+)\s+(\d+)", hdr)
if not m:
    print("FAIL: invalid PPM")
    sys.exit(1)
w, h = int(m.group(1)), int(m.group(2))
with open(p, "rb") as f:
    f.seek(m.end())
    pix = f.read(w * h * 3)
nb = sum(1 for i in range(0, len(pix), 3) if pix[i] | pix[i + 1] | pix[i + 2])
print(f"USB img screendump: {nb} non-black pixels")
if nb < 500:
    print("FAIL")
    sys.exit(1)
print("PASS: USB .img boot in QEMU")
PY
