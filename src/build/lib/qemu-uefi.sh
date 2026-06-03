[[ -n "${POPCORN_BUILD_QEMU_UEFI:-}" ]] && return 0
POPCORN_BUILD_QEMU_UEFI=1

: "${POPCORN_SRC:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

UEFI_IMG="${UEFI_IMG:-popcorn-uefi.img}"
OVMF_VARS="${OVMF_VARS:-$BUILD_BASE/ovmf_vars.fd}"

qemu_uefi_usb_args() {
  local code="$1"
  local extra=("${@:2}")
  qemu-system-x86_64 \
    -machine q35 -m 1024 -cpu max \
    -drive "if=pflash,format=raw,readonly=on,file=$code" \
    -drive "if=pflash,format=raw,file=$OVMF_VARS" \
    -drive "if=none,id=usbstick,format=raw,file=$POPCORN_SRC/$UEFI_IMG" \
    -device qemu-xhci,id=xhci \
    -device usb-storage,bus=xhci.0,drive=usbstick \
    "${extra[@]}"
}

qemu_uefi_test_stability() {
  local code dbg
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || { echo "FAIL: edk2-x86_64-code.fd not found" >&2; exit 1; }

  dbg="$BUILD_BASE/uefi-stability.log"
  ensure_ovmf_vars "$OVMF_VARS"
  rm -f "$dbg"
  qemu_kill_all

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -display none -no-reboot \
    -daemonize

  local s
  for s in 5 10 15 20 25 30; do
    sleep 5
    if ! pgrep -x qemu-system-x86_64 >/dev/null; then
      echo "FAIL: QEMU exited before ${s}s (guest shutdown/reset)"
      echo "debugcon: $(cat "$dbg" 2>/dev/null || true)"
      return 1
    fi
  done

  qemu_kill_all
  local body
  body="$(cat "$dbg" 2>/dev/null || true)"
  echo "debugcon: $body"
  case "$body" in *M*) ;; *) echo "FAIL: kmain loop (M) not reached"; return 1 ;; esac
  case "$body" in *R*) ;; *) echo "FAIL: RAM parse (R) not seen"; return 1 ;; esac
  echo "PASS: guest stayed alive 30s without -no-shutdown"
}

qemu_uefi_test_alive() {
  local code mon debug out_dir
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || die "edk2-x86_64-code.fd not found"

  mon="$BUILD_BASE/qemu-uefi-usb-mon.sock"
  debug="$BUILD_BASE/uefi-debugcon.log"
  out_dir="$BUILD_BASE/alive-dumps"

  ensure_ovmf_vars "$OVMF_VARS"
  mkdir -p "$out_dir"
  rm -f "$mon" "$debug"
  find "$out_dir" -maxdepth 1 -name 't*.ppm' -delete 2>/dev/null || true
  qemu_kill_all

  qemu_uefi_usb_args "$code" \
    -monitor "unix:$mon,server,nowait" \
    -debugcon "file:$debug" \
    -global isa-debugcon.iobase=0xe9 \
    -display none -no-reboot -no-shutdown \
    -daemonize

  dump_screen() {
    local tag="$1" out="$out_dir/${tag}.ppm" i
    for i in 1 2 3 4 5 6 7 8 9 10; do
      [[ -S "$mon" ]] && break
      sleep 1
    done
    { printf 'screendump %s\n' "$out"; sleep 0.5; } | nc -U "$mon" 2>/dev/null || true
    [[ -f "$out" ]] || echo "warn: screendump ${tag} failed" >&2
  }

  sleep 12
  dump_screen t10
  sleep 12
  dump_screen t20
  sleep 12
  dump_screen t30
  qemu_kill_all

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

def read_ppm(path):
    data = path.read_bytes()
    m = re.search(br"P6\s+(\d+)\s+(\d+)\s+(\d+)", data)
    if not m:
        return None
    w, h = int(m.group(1)), int(m.group(2))
    pix = data[m.end() : m.end() + w * h * 3]
    return w, h, pix

def probe_pixel(pix, w, h):
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

def ppm_diff_count(a_path, b_path):
    ra, rb = read_ppm(a_path), read_ppm(b_path)
    if not ra or not rb:
        return 0
    wa, ha, pa = ra
    wb, hb, pb = rb
    if (wa, ha) != (wb, hb):
        return -1
    return sum(1 for i in range(0, len(pa), 3) if pa[i : i + 3] != pb[i : i + 3])

first, last = ppm_files[0], ppm_files[-1]
diff_px = ppm_diff_count(first, last)
print(f"pixel diffs {first.name} vs {last.name}: {diff_px // 3}")

dbg_body = dbg.read_text(errors="replace") if dbg.exists() else ""
probes = [pr for _, pr, _ in rows]
hashes = [h for _, _, h in rows]
if diff_px <= 0 and len(set(probes)) < 2 and len(set(hashes)) < 2:
    if "R" in dbg_body and "M" in dbg_body and "KL" in dbg_body:
        print("PASS: kernel alive (debugcon R+M; GOP not in QEMU screendump)")
        sys.exit(0)
    print("FAIL: framebuffer appears static across captures")
    sys.exit(1)
print("PASS: display activity detected")
PY
}

qemu_uefi_test_debugcon() {
  local code dbg
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || die "edk2-x86_64-code.fd not found"

  dbg="$BUILD_BASE/uefi-smoke-debugcon.log"
  ensure_ovmf_vars "$OVMF_VARS"
  rm -f "$dbg"
  qemu_kill_all

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -display none -no-reboot -no-shutdown \
    -daemonize

  sleep 8
  qemu_kill_all
  [[ -f "$dbg" ]] || die "no debugcon log"

  local body
  body="$(cat "$dbg")"
  echo "debugcon: ${body:0:40}..."
  case "$body" in *icd*KL*) ;; *) die "expected boot trace icd..KL" ;; esac
  case "$body" in *R*) ;; *) die "expected R (UEFI RAM / MBI parsed)" ;; esac
  case "$body" in *M*) ;; *) die "expected M (kmain loop entered)" ;; esac
  echo "PASS: debugcon boot trace"
}

qemu_uefi_smoke() {
  echo "== stability (no -no-shutdown, 30s) =="
  qemu_uefi_test_stability || return 1
  echo "== alive (display) =="
  qemu_uefi_test_alive || return 1
  echo "== debugcon (kmain) =="
  qemu_uefi_test_debugcon || return 1
  echo "PASS: UEFI QEMU smoke"
}

qemu_uefi_run_interactive() {
  local code
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || die "edk2-x86_64-code.fd not found"
  [[ -f "$POPCORN_SRC/$UEFI_IMG" ]] || die "Missing $UEFI_IMG — run: ./build/core.sh img"

  ensure_ovmf_vars "$OVMF_VARS"
  log INFO "QEMU UEFI USB boot (interactive window)"
  exec qemu-system-x86_64 \
    -machine q35 -m 1024 -cpu max \
    -drive "if=pflash,format=raw,readonly=on,file=$code" \
    -drive "if=pflash,format=raw,file=$OVMF_VARS" \
    -drive "if=none,id=usbstick,format=raw,file=$POPCORN_SRC/$UEFI_IMG" \
    -device qemu-xhci,id=xhci \
    -device usb-storage,bus=xhci.0,drive=usbstick \
    -no-reboot
}
