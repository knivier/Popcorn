#!/usr/bin/env bash
# Boot popcorn-uefi.img and verify the alive counter (A:) changes over time.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$SRC_DIR"

IMG="${SRC_DIR}/popcorn-uefi.img"
VARS="${SRC_DIR}/buildbase/ovmf_vars.fd"
MON_SOCK="${SRC_DIR}/buildbase/qemu-uefi-usb-mon.sock"
DEBUG_LOG="${SRC_DIR}/buildbase/uefi-debugcon.log"
OUT_DIR="${SRC_DIR}/buildbase/alive-dumps"

find_edk_code() {
  for p in \
    /opt/homebrew/share/qemu/edk2-x86_64-code.fd \
    /opt/homebrew/Cellar/qemu/*/share/qemu/edk2-x86_64-code.fd; do
    [[ -f "$p" ]] && printf '%s' "$p" && return 0
  done
  return 1
}

bash build/macos.sh build >/dev/null
bash build/uefi.sh >/dev/null
bash build/img-uefi.sh >/dev/null

CODE="$(find_edk_code || true)"
[[ -n "$CODE" ]] || { echo "error: edk2-x86_64-code.fd not found" >&2; exit 1; }

mkdir -p buildbase "$OUT_DIR"
[[ -f "$VARS" ]] || dd if=/dev/zero of="$VARS" bs=1m count=4 status=none

rm -f "$MON_SOCK" "$DEBUG_LOG" "$OUT_DIR"/*.ppm
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
  -debugcon "file:$(cd "$SRC_DIR/buildbase" && pwd)/uefi-debugcon.log" \
  -global isa-debugcon.iobase=0xe9 \
  -display none \
  -no-reboot \
  -daemonize

dump_screen() {
  local tag="$1"
  local out="${OUT_DIR}/${tag}.ppm"
  { printf 'screendump %s\n' "$out"; sleep 1; } | nc -U "$MON_SOCK" 2>/dev/null || true
}

OUT_DIR="$(cd "$OUT_DIR" && pwd)"
MON_SOCK="$(cd "$(dirname "$MON_SOCK")" && pwd)/$(basename "$MON_SOCK")"

sleep 10
dump_screen "t10"
sleep 10
dump_screen "t20"
sleep 10
dump_screen "t30"

killall qemu-system-x86_64 2>/dev/null || true

python3 - <<'PY'
import re
import sys
from pathlib import Path

out_dir = Path("buildbase/alive-dumps")
ppm_files = sorted(out_dir.glob("t*.ppm"))
if len(ppm_files) < 2:
    print("FAIL: missing screendumps")
    sys.exit(1)

dbg = Path("buildbase/uefi-debugcon.log")
if dbg.exists():
    tail = dbg.read_text(errors="replace")
    print("debugcon:", tail[:24] + ("..." if len(tail) > 24 else ""))
else:
    print("debugcon: (missing)")

def read_ppm(path):
    data = path.read_bytes()
    m = re.search(br"P6\s+(\d+)\s+(\d+)\s+(\d+)", data)
    if not m:
        return None
    w, h = int(m.group(1)), int(m.group(2))
    pix = data[m.end() : m.end() + w * h * 3]
    return w, h, pix

def sample_bottom_band(w, h, pix):
    """Sample bottom 8% of screen for non-uniform pixels (heartbeat text)."""
    y0 = int(h * 0.92)
    samples = []
    for y in range(y0, h, max(1, (h - y0) // 8)):
        for x in range(0, w, max(1, w // 40)):
            i = (y * w + x) * 3
            samples.append(pix[i : i + 3])
    return samples

def probe_pixel(pix, w, h):
    """Sample near centered text panel top-left (matches fb_origin + 4)."""
    x = int(w * 0.08) + 4
    y = int(h * 0.08) + 4
    i = (y * w + x) * 3
    return pix[i], pix[i + 1], pix[i + 2]

def band_hash(pix, w, h):
    y0 = int(h * 0.92)
    hsh = probe_pixel(pix, w, h)[0] + probe_pixel(pix, w, h)[1] * 256
    for y in range(y0, h, 4):
        for x in range(52 * w // 80, w, 8):
            i = (y * w + x) * 3
            hsh = (hsh * 131) + pix[i] + pix[i + 1] + pix[i + 2]
    return hsh

rows = []
for p in ppm_files:
    r = read_ppm(p)
    if not r:
        print(f"FAIL: bad ppm {p}")
        sys.exit(1)
    w, h, pix = r
    rows.append((p.name, probe_pixel(pix, w, h), band_hash(pix, w, h)))

print("probe (panel TL) RGB + band hash:")
for name, pr, h in rows:
    print(f"  {name}: probe={pr} hash_tail={h % 1000000}")
hashes = [h for _, _, h in rows]

def ppm_diff_count(a_path, b_path):
    ra, rb = read_ppm(a_path), read_ppm(b_path)
    if not ra or not rb:
        return 0
    wa, ha, pa = ra
    wb, hb, pb = rb
    if (wa, ha) != (wb, hb) or len(pa) != len(pb):
        return -1
    return sum(1 for i in range(0, len(pa), 3) if pa[i : i + 3] != pb[i : i + 3])

first, last = ppm_files[0], ppm_files[-1]
diff_px = ppm_diff_count(first, last)
print(f"pixel diffs {first.name} vs {last.name}: {diff_px // 3}")

probes = [pr for _, pr, _ in rows]
hashes = [h for _, _, h in rows]
if diff_px <= 0 and len(set(probes)) < 2 and len(set(hashes)) < 2:
    print("FAIL: framebuffer appears static across captures")
    sys.exit(1)
print("PASS: display activity detected")
PY
