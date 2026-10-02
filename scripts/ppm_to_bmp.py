#!/usr/bin/env python3
"""Convert target/verify-screen.ppm to a Windows-friendly BMP."""
from pathlib import Path
import struct

root = Path(__file__).resolve().parents[1]
ppm_path = root / "target" / "verify-screen.ppm"
bmp_path = root / "target" / "verify-screen.bmp"
data = ppm_path.read_bytes()
parts = data.split(b"\n", 3)
w, h = map(int, parts[1].split())
pix = parts[3]
row = (w * 3 + 3) // 4 * 4
raw = bytearray(row * h)
for y in range(h):
    src = y * w * 3
    dst = (h - 1 - y) * row
    for x in range(w):
        i = src + x * 3
        j = dst + x * 3
        # PPM is RGB; BMP is BGR
        raw[j] = pix[i + 2]
        raw[j + 1] = pix[i + 1]
        raw[j + 2] = pix[i]
bf = bytearray(14 + 40 + len(raw))
bf[0:2] = b"BM"
struct.pack_into("<IHHI", bf, 2, 14 + 40 + len(raw), 0, 0, 14 + 40)
struct.pack_into("<IIIHHIIIIII", bf, 14, 40, w, h, 1, 24, 0, len(raw), 2835, 2835, 0, 0)
bf[54:] = raw
bmp_path.write_bytes(bf)
print(f"wrote {bmp_path} ({w}x{h})")
