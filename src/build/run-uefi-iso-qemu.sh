#!/usr/bin/env bash
# Boot popcorn-uefi.iso in QEMU (OVMF).
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

ISO="${SRC_DIR}/popcorn-uefi.iso"
VARS="${SRC_DIR}/buildbase/ovmf_vars.fd"
MON_SOCK="${SRC_DIR}/buildbase/qemu-uefi-iso-mon.sock"
SCREEN="${SRC_DIR}/buildbase/uefi-iso-screen.ppm"

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

bash build/iso-uefi.sh

CODE="$(find_edk_code || true)"
[[ -n "$CODE" ]] || { echo "error: edk2-x86_64-code.fd not found" >&2; exit 1; }
[[ -f "$ISO" ]] || { echo "error: missing $ISO" >&2; exit 1; }

mkdir -p buildbase
if [[ ! -f "$VARS" ]]; then
  dd if=/dev/zero of="$VARS" bs=1m count=4 status=none
fi

rm -f "$MON_SOCK" "$SCREEN"
killall qemu-system-x86_64 2>/dev/null || true

echo "Starting QEMU UEFI ISO test..."
echo "  ISO:  $ISO"

qemu-system-x86_64 \
  -machine q35 \
  -m 1024 \
  -cpu max \
  -drive "if=pflash,format=raw,readonly=on,file=$CODE" \
  -drive "if=pflash,format=raw,file=$VARS" \
  -cdrom "$ISO" \
  -boot d \
  -monitor "unix:$MON_SOCK,server,nowait" \
  -display none \
  -no-reboot \
  -daemonize

sleep 20
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
if nb < 500:
    print("FAIL: screen appears blank (kernel may not have reached console)")
    sys.exit(1)
print("PASS: UEFI ISO produced visible framebuffer output in QEMU")
PY
