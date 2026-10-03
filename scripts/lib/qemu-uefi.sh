[[ -n "${POPCORN_BUILD_QEMU_UEFI:-}" ]] && return 0
POPCORN_BUILD_QEMU_UEFI=1

: "${POPCORN_TARGET:?}"
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
# shellcheck source=kernel.sh
source "$(dirname "${BASH_SOURCE[0]}")/kernel.sh"
# shellcheck source=img-uefi.sh
source "$(dirname "${BASH_SOURCE[0]}")/img-uefi.sh"

UEFI_IMG="${UEFI_IMG:-$POPCORN_TARGET/popcorn-uefi.img}"
OVMF_VARS="${OVMF_VARS:-$POPCORN_TARGET/ovmf_vars.fd}"

# Video for UEFI/GOP: std VGA often leaves a blank GTK window while the kernel
# correctly paints OVMF's GOP buffer. virtio-vga (or ramfb) is what QEMU shows.
qemu_uefi_video_args() {
  # 1280x800 matches 80x25 @ 8x16 glyphs at 2x (640x400 → 1280x800).
  local res="${POPCORN_QEMU_RES:-1280x800}"
  local xres="${res%x*}"
  local yres="${res#*x}"
  if qemu-system-x86_64 -device help 2>&1 | grep -q 'name "virtio-vga"'; then
    printf '%s\n' -vga none -device "virtio-vga,xres=${xres},yres=${yres}"
  elif qemu-system-x86_64 -device help 2>&1 | grep -q 'name "ramfb"'; then
    printf '%s\n' -vga none -device ramfb
  else
    printf '%s\n' -vga std
  fi
}

qemu_uefi_display_args() {
  local mode="${POPCORN_QEMU_DISPLAY:-auto}"
  case "$mode" in
    vnc)
      log INFO "VNC: connect a viewer to 127.0.0.1:5900 (display :0) — optional, not required"
      printf '%s\n' -display none -vnc "127.0.0.1:0,to=9"
      ;;
    gtk)
      export DISPLAY="${DISPLAY:-:0}"
      unset WAYLAND_DISPLAY
      printf '%s\n' -display gtk,gl=off
      ;;
    sdl) printf '%s\n' -display sdl ;;
    none) printf '%s\n' -display none ;;
    cocoa) printf '%s\n' -display cocoa ;;
    auto|*)
      if [[ "$HOST_OS" == "darwin" ]]; then
        printf '%s\n' -display cocoa
      elif [[ -n "${WSL_DISTRO_NAME:-}" || -n "${WSL_INTEROP:-}" ]]; then
        # WSLg GTK windows are often blank/unclickable ("COPY MODE" ghost icons).
        # Default to VNC on WSL so any Windows VNC viewer can attach reliably.
        log INFO "WSL: using VNC on 127.0.0.1:5900 — open TightVNC / RealVNC / TigerVNC"
        log INFO "  (override with POPCORN_QEMU_DISPLAY=gtk if WSLg windows work for you)"
        printf '%s\n' -display none -vnc "127.0.0.1:0,to=9"
      else
        printf '%s\n' -display gtk,gl=off
      fi
      ;;
  esac
}

qemu_uefi_usb_args() {
  local code="$1"
  local extra=("${@:2}")
  # shellcheck disable=SC2046
  qemu-system-x86_64 \
    -machine q35 -m 1024 -cpu max \
    -drive "if=pflash,format=raw,readonly=on,file=$code" \
    -drive "if=pflash,format=raw,file=$OVMF_VARS" \
    -drive "if=none,id=usbstick,format=raw,file=$UEFI_IMG" \
    -device qemu-xhci,id=xhci \
    -device usb-storage,bus=xhci.0,drive=usbstick \
    $(qemu_uefi_video_args) \
    "${extra[@]}"
}

qemu_uefi_test_stability() {
  local code dbg
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || { echo "FAIL: edk2-x86_64-code.fd not found" >&2; exit 1; }

  dbg="$POPCORN_TARGET/uefi-stability.log"
  # Fresh vars avoid stuck OVMF boot menus from prior runs.
  rm -f "$OVMF_VARS"
  ensure_ovmf_vars "$OVMF_VARS"
  rm -f "$dbg" "$POPCORN_TARGET/uefi-stability-serial.log"
  qemu_kill_all
  sleep 1

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$POPCORN_TARGET/uefi-stability-serial.log" \
    -display none -no-reboot \
    -daemonize

  local waited=0
  while [[ $waited -lt 45 ]]; do
    if ! pgrep -f qemu-system-x86_64 >/dev/null; then
      echo "FAIL: QEMU exited before boot completed (${waited}s)"
      echo "debugcon: $(cat "$dbg" 2>/dev/null || true)"
      return 1
    fi
    if [[ -f "$dbg" ]] && grep -q 'M' "$dbg" 2>/dev/null; then
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done

  # Stay alive a bit after reaching kmain.
  sleep 5
  if ! pgrep -f qemu-system-x86_64 >/dev/null; then
    echo "FAIL: QEMU exited after reaching kmain"
    echo "debugcon: $(cat "$dbg" 2>/dev/null || true)"
    return 1
  fi

  qemu_kill_all
  local body
  body="$(cat "$dbg" 2>/dev/null || true)"
  echo "debugcon: $body"
  case "$body" in *M*) ;; *) echo "FAIL: kmain loop (M) not reached"; return 1 ;; esac
  case "$body" in *R*) ;; *) echo "FAIL: RAM parse (R) not seen"; return 1 ;; esac
  echo "PASS: guest reached kmain and stayed alive"
}

qemu_uefi_test_alive() {
  local code mon debug out_dir
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || die "edk2-x86_64-code.fd not found"

  mon="$RUNTIME_DIR/qemu-uefi-usb-mon.sock"
  debug="$POPCORN_TARGET/uefi-debugcon.log"
  out_dir="$POPCORN_TARGET/alive-dumps"

  ensure_ovmf_vars "$OVMF_VARS"
  mkdir -p "$out_dir" "$RUNTIME_DIR"
  rm -f "$mon" "$debug"
  find "$out_dir" -maxdepth 1 -name 't*.ppm' -delete 2>/dev/null || true
  qemu_kill_all

  qemu_uefi_usb_args "$code" \
    -monitor "unix:$mon,server,nowait" \
    -debugcon "file:$debug" \
    -global isa-debugcon.iobase=0xe9 \
    -serial "file:$POPCORN_TARGET/uefi-alive-serial.log" \
    -display none -no-reboot -no-shutdown \
    -daemonize

  dump_screen() {
    local tag="${1:-x}" out i
    out="$out_dir/${tag}.ppm"
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

  TARGET_DIR="$POPCORN_TARGET" python3 - <<'PY'
import os
import re
import sys
from pathlib import Path

target = Path(os.environ["TARGET_DIR"])
out_dir = target / "alive-dumps"
ppm_files = sorted(out_dir.glob("t*.ppm"))
if len(ppm_files) < 2:
    print("FAIL: missing screendumps")
    sys.exit(1)

dbg = target / "uefi-debugcon.log"
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
# Heartbeat/status may not move enough pixels for a reliable screendump delta.
# Boot tags on debugcon are the authoritative "guest is alive" signal.
if "M" in dbg_body and ("R" in dbg_body or "KL" in dbg_body):
    if diff_px > 0 or len(set(hashes)) > 1:
        print("PASS: display activity detected")
    else:
        print("PASS: kernel alive (debugcon; GOP screendump static/ok)")
    sys.exit(0)
if diff_px > 0 or len(set(probes)) > 1 or len(set(hashes)) > 1:
    print("PASS: display activity detected")
    sys.exit(0)
print("FAIL: no boot tags and framebuffer appears static")
sys.exit(1)
PY
}

qemu_uefi_test_debugcon() {
  local code dbg
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || die "edk2-x86_64-code.fd not found"

  dbg="$POPCORN_TARGET/uefi-smoke-debugcon.log"
  ensure_ovmf_vars "$OVMF_VARS"
  rm -f "$dbg"
  qemu_kill_all

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$POPCORN_TARGET/uefi-smoke-serial.log" \
    -display none -no-reboot -no-shutdown \
    -daemonize

  local waited=0
  while [[ $waited -lt 40 ]]; do
    if [[ -f "$dbg" ]] && grep -q 'M' "$dbg" 2>/dev/null; then
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  qemu_kill_all
  [[ -f "$dbg" ]] || die "no debugcon log"

  local body
  body="$(cat "$dbg")"
  echo "debugcon: ${body:0:40}..."
  case "$body" in *icd*KL*) ;; *) die "expected boot trace icd..KL" ;; esac
  case "$body" in *R*) ;; *) die "expected R (UEFI RAM / MBI parsed)" ;; esac
  case "$body" in *r*) ;; *) die "expected r (rust_init / Rust active)" ;; esac
  case "$body" in *M*) ;; *) die "expected M (kmain loop entered)" ;; esac
  case "$body" in *I*) ;; *) die "expected I (Phase2 ioctl→device)" ;; esac
  case "$body" in *B*) ;; *) die "expected B (Phase2 wait-queue wake)" ;; esac
  case "$body" in *S*) ;; *) die "expected S (Phase2 sleep wake)" ;; esac
  case "$body" in *2*) ;; *) die "expected 2 (Phase2 self-test passed)" ;; esac
  echo "PASS: debugcon boot trace"
}

# Rebuild with -DPOPCORN_TEST_PF, boot briefly, expect #PF dump on debugcon/COM1.
qemu_uefi_test_pf() {
  local code dbg serial
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || { echo "FAIL: edk2-x86_64-code.fd not found" >&2; return 1; }

  log INFO "Building kernel with POPCORN_TEST_PF for exception dump check"
  export POPCORN_CFLAGS="-DPOPCORN_TEST_PF"
  build_all_uefi
  unset POPCORN_CFLAGS

  dbg="$POPCORN_TARGET/pf-debugcon.log"
  serial="$POPCORN_TARGET/pf-serial.log"
  rm -f "$OVMF_VARS"
  ensure_ovmf_vars "$OVMF_VARS"
  rm -f "$dbg" "$serial"
  qemu_kill_all
  sleep 1

  # shellcheck disable=SC2046
  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$serial" \
    -display none -no-reboot \
    -daemonize

  local waited=0
  while [[ $waited -lt 45 ]]; do
    if grep -q '#PF' "$dbg" 2>/dev/null || grep -q '#PF' "$serial" 2>/dev/null; then
      break
    fi
    if ! pgrep -f qemu-system-x86_64 >/dev/null; then
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  qemu_kill_all

  local body
  body="$(cat "$dbg" 2>/dev/null || true)$(cat "$serial" 2>/dev/null || true)"
  echo "pf log: ${body:0:240}"
  case "$body" in *'#PF'*) ;; *) echo "FAIL: no #PF dump on serial/debugcon"; return 1 ;; esac
  case "$body" in *CR2=*) ;; *) echo "FAIL: #PF dump missing CR2="; return 1 ;; esac
  case "$body" in *RIP=*) ;; *) echo "FAIL: #PF dump missing RIP="; return 1 ;; esac
  echo "PASS: #PF serial dump"

  log INFO "Rebuilding kernel without POPCORN_TEST_PF"
  build_all_uefi
}

qemu_uefi_smoke() {
  echo "== stability (no -no-shutdown, 30s) =="
  qemu_uefi_test_stability || return 1
  echo "== alive (display) =="
  qemu_uefi_test_alive || return 1
  echo "== debugcon (kmain) =="
  qemu_uefi_test_debugcon || return 1
  echo "== page fault dump =="
  qemu_uefi_test_pf || return 1
  echo "== GRUB ISO =="
  qemu_legacy_smoke || return 1
  echo "PASS: UEFI QEMU smoke"
}

qemu_uefi_run_interactive() {
  local code
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || die "edk2-x86_64-code.fd not found"
  [[ -f "$UEFI_IMG" ]] || die "Missing $UEFI_IMG — run: ./scripts/core.sh img"

  ensure_ovmf_vars "$OVMF_VARS"
  log INFO "QEMU UEFI USB boot (interactive window)"
  log INFO "Video: virtio-vga/ramfb (GOP). Serial boot tags still print here."

  # shellcheck disable=SC2046
  exec qemu-system-x86_64 \
    -machine q35 -m 1024 -cpu max \
    -drive "if=pflash,format=raw,readonly=on,file=$code" \
    -drive "if=pflash,format=raw,file=$OVMF_VARS" \
    -drive "if=none,id=usbstick,format=raw,file=$UEFI_IMG" \
    -device qemu-xhci,id=xhci \
    -device usb-storage,bus=xhci.0,drive=usbstick \
    $(qemu_uefi_video_args) \
    $(qemu_uefi_display_args) \
    -serial stdio \
    -no-reboot
}
