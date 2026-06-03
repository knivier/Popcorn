#!/usr/bin/env bash
# Boot the native UEFI loader in QEMU (OVMF + FAT ESP).
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

ESP="${SRC_DIR}/uefi_usb"
VARS="${SRC_DIR}/buildbase/ovmf_vars.fd"
MON_SOCK="${SRC_DIR}/buildbase/qemu-uefi-mon.sock"
SCREEN="${SRC_DIR}/buildbase/uefi-screen.ppm"

have() { command -v "$1" >/dev/null 2>&1; }

find_edk_code() {
  local p
  for p in \
    /opt/homebrew/share/qemu/edk2-x86_64-code.fd \
    /opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd; do
    if [[ -f "$p" ]]; then
      printf '%s' "$p"
      return 0
    fi
  done
  return 1
}

send_monitor_cmd() {
  local cmd="$1"
  if have nc; then
    printf '%s\n' "$cmd" | nc -U "$MON_SOCK" >/dev/null 2>&1 || return 1
    return 0
  fi
  return 1
}

bash build/uefi.sh

CODE="$(find_edk_code || true)"
[[ -n "$CODE" ]] || { echo "error: edk2-x86_64-code.fd not found (install qemu via Homebrew)" >&2; exit 1; }
[[ -f "$ESP/EFI/BOOT/BOOTX64.EFI" ]] || { echo "error: missing $ESP/EFI/BOOT/BOOTX64.EFI" >&2; exit 1; }

mkdir -p buildbase
# q35 pflash is 8 MiB total; code.fd is ~3.5 MiB.
if [[ ! -f "$VARS" ]]; then
  dd if=/dev/zero of="$VARS" bs=1m count=4 status=none
fi

rm -f "$MON_SOCK" "$SCREEN"
killall qemu-system-x86_64 2>/dev/null || true

echo "Starting QEMU UEFI test..."
echo "  CODE: $CODE"
echo "  ESP:  $ESP"

qemu-system-x86_64 \
  -machine q35 \
  -m 512 \
  -cpu max \
  -drive "if=pflash,format=raw,readonly=on,file=$CODE" \
  -drive "if=pflash,format=raw,file=$VARS" \
  -drive "format=raw,file=fat:rw:$ESP" \
  -monitor "unix:$MON_SOCK,server,nowait" \
  -display none \
  -no-reboot \
  -daemonize

sleep 8
send_monitor_cmd "screendump $SCREEN" || true
sleep 1
killall qemu-system-x86_64 2>/dev/null || true

if [[ ! -f "$SCREEN" ]]; then
  echo "FAIL: no screendump produced"
  exit 1
fi

python3 - <<PY
import re, sys
p = "$SCREEN"
with open(p, "rb") as f:
    hdr = f.read(64)
m = re.search(br"P6\s+(\d+)\s+(\d+)\s+(\d+)", hdr)
if not m:
    print("FAIL: invalid PPM header")
    sys.exit(1)
w, h = int(m.group(1)), int(m.group(2))
start = m.end()
with open(p, "rb") as f:
    f.seek(start)
    pix = f.read(w * h * 3)
nb = sum(1 for i in range(0, len(pix), 3) if pix[i] | pix[i + 1] | pix[i + 2])
ratio = nb / (w * h)
print(f"Screendump: {w}x{h}, non-black pixels: {nb} ({ratio * 100:.2f}%)")
if nb < 100:
    print("FAIL: screen appears blank in QEMU")
    sys.exit(1)
print("PASS: UEFI loader produced visible framebuffer output in QEMU")
PY

echo ""
echo "Interactive test (window):"
echo "  qemu-system-x86_64 -machine q35 -m 512 -cpu max \\"
echo "    -drive if=pflash,format=raw,readonly=on,file=$CODE \\"
echo "    -drive if=pflash,format=raw,file=$VARS \\"
echo "    -drive format=raw,file=fat:rw:$ESP"
