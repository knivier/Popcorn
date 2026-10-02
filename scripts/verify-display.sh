#!/usr/bin/env bash
# Headless verify: UEFI + virtio-vga produces a non-blank screendump.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export POPCORN_ROOT="$ROOT"
export POPCORN_SRC="$ROOT/src"
export POPCORN_TARGET="$ROOT/target"
export POPCORN_SCRIPTS="$ROOT/scripts"

# shellcheck source=lib/common.sh
source "$ROOT/scripts/lib/common.sh"
# shellcheck source=lib/kernel.sh
source "$ROOT/scripts/lib/kernel.sh"
# shellcheck source=lib/img-uefi.sh
source "$ROOT/scripts/lib/img-uefi.sh"
# shellcheck source=lib/qemu-uefi.sh
source "$ROOT/scripts/lib/qemu-uefi.sh"

killall qemu-system-x86_64 2>/dev/null || true
sleep 1
code="$(find_edk_code)"
ensure_ovmf_vars "$OVMF_VARS"
mon="$RUNTIME_DIR/verify-mon.sock"
ppm="$POPCORN_TARGET/verify-screen.ppm"
dbg="$POPCORN_TARGET/verify-debugcon.log"
mkdir -p "$RUNTIME_DIR" "$POPCORN_TARGET"
rm -f "$mon" "$ppm" "$dbg"

# shellcheck disable=SC2046
qemu-system-x86_64 \
  -machine q35 -m 1024 -cpu max \
  -drive "if=pflash,format=raw,readonly=on,file=$code" \
  -drive "if=pflash,format=raw,file=$OVMF_VARS" \
  -drive "if=none,id=usbstick,format=raw,file=$UEFI_IMG" \
  -device qemu-xhci,id=xhci \
  -device usb-storage,bus=xhci.0,drive=usbstick \
  $(qemu_uefi_video_args) \
  -display none \
  -monitor "unix:$mon,server,nowait" \
  -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
  -no-reboot -no-shutdown -daemonize

sleep 12
for _ in 1 2 3 4 5 6 7 8 9 10; do
  [[ -S "$mon" ]] && break
  sleep 1
done
{ printf 'screendump %s\n' "$ppm"; sleep 0.5; } | nc -U "$mon" 2>/dev/null || true
killall qemu-system-x86_64 2>/dev/null || true

echo "debugcon=$(cat "$dbg" 2>/dev/null || true)"
echo "video=$(qemu_uefi_video_args | tr '\n' ' ')"
if [[ ! -f "$ppm" ]]; then
  echo "FAIL: no screendump at $ppm"
  exit 1
fi
python3 - <<PY
from pathlib import Path
p = Path("$ppm")
data = p.read_bytes()
parts = data.split(b"\n", 3)
pix = parts[-1] if len(parts) >= 4 else data
nonzero = sum(1 for b in pix[: min(len(pix), 800000)] if b != 0)
print(f"ppm={p} bytes={len(data)} nonzero_sample={nonzero}")
if nonzero < 100:
    raise SystemExit("FAIL: screendump looks blank")
print("PASS: screendump has visible pixels")
PY
