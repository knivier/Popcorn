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
# QEMU scratch disks — never flash these.
UNSAFE_DIR="${UNSAFE_DIR:-$POPCORN_TARGET/UNSAFE}"
DATA_IMG="${DATA_IMG:-$UNSAFE_DIR/UNSAFE-qemu-virtio.img}"
INTERNAL_IMG="${INTERNAL_IMG:-$UNSAFE_DIR/UNSAFE-qemu-nvme.img}"
USB_DATA_IMG="${USB_DATA_IMG:-$UNSAFE_DIR/UNSAFE-qemu-usb-data.img}"
OVMF_VARS="${OVMF_VARS:-$POPCORN_TARGET/ovmf_vars.fd}"

# Ensure a raw image exists (never the Windows host drive).
ensure_raw_img() {
  local path="$1" mib="$2" label="$3"
  mkdir -p "$(dirname "$path")"
  if [[ ! -f "$UNSAFE_DIR/DO-NOT-FLASH.txt" ]]; then
    mkdir -p "$UNSAFE_DIR"
    cat > "$UNSAFE_DIR/DO-NOT-FLASH.txt" <<'EOF'
UNSAFE — QEMU-only images. Do not flash any file in this directory to USB or a real disk.
The only flashable artifact is ../popcorn-uefi.img after a normal ./scripts/core.sh all
(without POPCORN_TEST_INSTALL).
EOF
  fi
  if [[ ! -f "$path" ]]; then
    log INFO "Creating $label: $path (${mib} MiB)"
    dd if=/dev/zero of="$path" bs=1M count="$mib" status=none 2>/dev/null \
      || dd if=/dev/zero of="$path" bs=1048576 count="$mib" 2>/dev/null
  fi
}

# Guest disks must be regular files under $POPCORN_TARGET. Never PhysicalDrive,
# never /dev/sd*, never the Windows NVMe.
qemu_assert_guest_file() {
  local path="$1" what="$2"
  local abs target
  if [[ -z "$path" ]]; then
    die "REFUSING QEMU $what: empty path"
  fi
  case "$path" in
    *PhysicalDrive*|*PHYSICALDRIVE*|\\\\.\\*|/dev/sd*|/dev/hd*|/dev/nvme*|/dev/vd*|/dev/xvd*|/dev/mapper/*|/dev/disk/*)
      die "REFUSING QEMU $what: host disk passthrough is forbidden ($path)"
      ;;
  esac
  if [[ -b "$path" || -c "$path" ]]; then
    die "REFUSING QEMU $what: $path is a block/char device"
  fi
  if [[ ! -f "$path" ]]; then
    die "REFUSING QEMU $what: not a regular file: $path"
  fi
  abs="$(cd "$(dirname "$path")" && pwd)/$(basename "$path")"
  target="$(cd "$POPCORN_TARGET" && pwd)"
  case "$abs" in
    "$target"/*) ;;
    *)
      die "REFUSING QEMU $what: $abs is outside target/ (host disks are never attached)"
      ;;
  esac
}

qemu_assert_boot_img() {
  [[ -f "$UEFI_IMG" ]] || die "Missing $UEFI_IMG — run: ./scripts/core.sh img"
  qemu_assert_guest_file "$UEFI_IMG" "boot image"
  case "$UEFI_IMG" in
    *UNSAFE*)
      if [[ "${POPCORN_CFLAGS:-}" == *POPCORN_TEST_INSTALL* ]]; then
        log WARNING "QEMU test-install: UNSAFE image (scratch files under target/ only)"
        return 0
      fi
      # core.sh test-shell: serial-console test image. Still subject to the
      # auto-install check below.
      if [[ "${POPCORN_SHELL_TEST:-}" == 1 && "$(basename "$UEFI_IMG")" == UNSAFE-popcorn-shelltest.img ]]; then
        :
      else
        die "REFUSING to boot $UEFI_IMG. Interactive/flash path is target/popcorn-uefi.img"
      fi
      ;;
  esac
  if grep -aE -q 'install-selftest|rust_test_disk_install' "$UEFI_IMG"; then
    die "REFUSING to boot auto-install image $UEFI_IMG"
  fi
}

# Data disks for the picker (all under target/UNSAFE — never flash):
#   usb-storage (xHCI, UNSAFE-qemu-usb-data.img) → guest usb MSC
#   virtio-blk → guest vda / usb0 stand-in
#   NVMe       → guest nvme0 (Internal, writes impossible)
# Call from the parent shell (not inside $()) so die() actually stops QEMU.
qemu_prepare_scratch_disks() {
  ensure_raw_img "$DATA_IMG" 64 "USB/boot stand-in disk"
  ensure_raw_img "$USB_DATA_IMG" 64 "USB mass-storage data disk"
  ensure_raw_img "$INTERNAL_IMG" 128 "internal NVMe disk"
  qemu_assert_guest_file "$DATA_IMG" "virtio scratch"
  qemu_assert_guest_file "$USB_DATA_IMG" "usb-storage scratch"
  qemu_assert_guest_file "$INTERNAL_IMG" "emulated NVMe scratch"
  if [[ -n "${MODERN_IMG:-}" ]]; then
    ensure_raw_img "$MODERN_IMG" 8 "modern-only virtio-blk (framework probe)"
    qemu_assert_guest_file "$MODERN_IMG" "modern virtio scratch"
  fi
}

# Fills QEMU_DATA_DISK_ARGS. An array (not printf + unquoted $(...)) keeps disk
# paths intact when the checkout path contains spaces.
# virtio-net on QEMU user-mode networking (10.0.2.x, gateway 10.0.2.2, DNS 10.0.2.3).
# POPCORN_QEMU_NET=off leaves the machine without a NIC (net: none). An explicit
# -netdev also removes QEMU's default e1000/e1000e. MSI-X is on by default (M2
# hybrid interrupts); POPCORN_NET_VECTORS=0 hides MSI-X so the driver polls, and
# POPCORN_NET_EVENT_IDX=off exercises the no-EVENT_IDX notification path.
qemu_net_args() {
  QEMU_NET_ARGS=()
  case "${POPCORN_QEMU_NET:-user}" in
    off) QEMU_NET_ARGS=(-nic none) ;;
    *)
      local fwd=""
      # Host 127.0.0.1:$POPCORN_NET_HOSTFWD_UDP -> guest UDP 5555 (the shell tests'
      # `udp -echo 5555` round trip); loopback only, never exposed beyond the host.
      if [[ -n "${POPCORN_NET_HOSTFWD_UDP:-}" ]]; then
        fwd=",hostfwd=udp:127.0.0.1:${POPCORN_NET_HOSTFWD_UDP}-:5555"
      fi
      QEMU_NET_ARGS=(
        -netdev "user,id=pnet0,ipv6=off${fwd}"
        -device "virtio-net-pci,netdev=pnet0,id=pnic0,mac=52:54:00:12:34:56,disable-legacy=on,romfile=${POPCORN_NET_VECTORS:+,vectors=$POPCORN_NET_VECTORS}${POPCORN_NET_EVENT_IDX:+,event_idx=$POPCORN_NET_EVENT_IDX}"
      )
      if [[ -n "${POPCORN_NET_PCAP:-}" ]]; then
        QEMU_NET_ARGS+=(-object "filter-dump,id=pdump0,netdev=pnet0,file=$POPCORN_NET_PCAP")
      fi
      ;;
  esac
}

qemu_uefi_data_disk_args() {
  QEMU_DATA_DISK_ARGS=(
    -drive "if=none,id=popmsc,format=raw,file=$USB_DATA_IMG"
    -device "usb-storage,bus=xhci.0,drive=popmsc"
    -drive "if=none,id=popusb,format=raw,file=$DATA_IMG"
    -device "virtio-blk-pci,drive=popusb,disable-modern=${POPCORN_BLK_DISABLE_MODERN:-on}"
    -drive "if=none,id=popnvme,format=raw,file=$INTERNAL_IMG"
    -device "nvme,drive=popnvme,serial=POPCORN1"
  )
  qemu_net_args
  QEMU_DATA_DISK_ARGS+=("${QEMU_NET_ARGS[@]}")
  # Shell tests only: a modern-only (0x1042) virtio-blk that the legacy block
  # driver ignores; `virtio -probe` drives it through the new driver framework.
  if [[ -n "${MODERN_IMG:-}" ]]; then
    QEMU_DATA_DISK_ARGS+=(
      -drive "if=none,id=popmodern,format=raw,file=$MODERN_IMG"
      -device "virtio-blk-pci,drive=popmodern,disable-legacy=on,disable-modern=off"
    )
  fi
}

# Video for UEFI/GOP: std VGA often leaves a blank GTK window while the kernel
# correctly paints OVMF's GOP buffer. virtio-vga (or ramfb) is what QEMU shows.
qemu_uefi_video_args() {
  # Temporary: 720p so VNC stays compact/readable. Override with POPCORN_QEMU_RES.
  local res="${POPCORN_QEMU_RES:-1280x720}"
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
  qemu_assert_boot_img
  qemu_assert_guest_file "$OVMF_VARS" "OVMF vars"
  qemu_prepare_scratch_disks
  qemu_uefi_data_disk_args
  qemu_kill_guest
  # Video args carry no paths, so word-splitting them is safe.
  # shellcheck disable=SC2046
  qemu-system-x86_64 \
    -machine q35 -accel kvm -accel tcg -m 4096 -cpu max \
    -drive "if=pflash,format=raw,readonly=on,file=$code" \
    -drive "if=pflash,format=raw,file=$OVMF_VARS" \
    -drive "if=none,id=usbstick,format=raw,file=$UEFI_IMG${QEMU_BOOT_DRIVE_OPTS:-}" \
    -device qemu-xhci,id=xhci \
    -device usb-storage,bus=xhci.0,drive=usbstick,bootindex=0 \
    "${QEMU_DATA_DISK_ARGS[@]}" \
    $(qemu_uefi_video_args) \
    -pidfile "$QEMU_PIDFILE" \
    "${extra[@]}"
}

qemu_uefi_test_stability() {
  local code dbg serial boot_timeout
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || { echo "FAIL: edk2-x86_64-code.fd not found" >&2; exit 1; }

  dbg="$POPCORN_TARGET/uefi-stability.log"
  serial="$POPCORN_TARGET/uefi-stability-serial.log"
  boot_timeout="${POPCORN_QEMU_BOOT_TIMEOUT:-120}"
  # Fresh vars avoid stuck OVMF boot menus from prior runs.
  rm -f "$OVMF_VARS"
  ensure_ovmf_vars "$OVMF_VARS"
  rm -f "$dbg" "$serial"
  qemu_kill_guest
  sleep 1

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$serial" \
    -display none -no-reboot \
    -daemonize

  local waited=0
  while [[ $waited -lt "$boot_timeout" ]]; do
    if ! qemu_guest_alive; then
      echo "FAIL: QEMU exited before boot completed (${waited}s)"
      echo "debugcon: $(cat "$dbg" 2>/dev/null || true)"
      echo "serial: $(cat "$serial" 2>/dev/null || true)"
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
  if ! qemu_guest_alive; then
    echo "FAIL: QEMU exited after reaching kmain"
    echo "debugcon: $(cat "$dbg" 2>/dev/null || true)"
    echo "serial: $(cat "$serial" 2>/dev/null || true)"
    return 1
  fi

  qemu_kill_guest
  local body
  body="$(cat "$dbg" 2>/dev/null || true)"
  echo "debugcon: $body"
  case "$body" in
    *M*) ;;
    *)
      echo "FAIL: kmain loop (M) not reached after ${boot_timeout}s"
      echo "serial: $(cat "$serial" 2>/dev/null || true)"
      return 1
      ;;
  esac
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
  qemu_kill_guest

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
  qemu_kill_guest

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
    # Exactly one whitespace byte separates maxval from the raster.
    m = re.search(br"P6\s+(\d+)\s+(\d+)\s+(\d+)\s", data)
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
print(f"pixel diffs {first.name} vs {last.name}: {diff_px}")

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
  qemu_kill_guest

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$POPCORN_TARGET/uefi-smoke-serial.log" \
    -display none -no-reboot -no-shutdown \
    -daemonize

  # Wait for Phase2 to finish ('2' after 'M') or for an exception dump.
  local waited=0 body=""
  while [[ $waited -lt 40 ]]; do
    body="$(cat "$dbg" 2>/dev/null || true)"
    case "$body" in *M*2*) break ;; esac
    boot_log_has_exception "$body" && break
    qemu_guest_alive || break
    sleep 1
    waited=$((waited + 1))
  done
  qemu_kill_guest
  [[ -f "$dbg" ]] || die "no debugcon log"

  body="$(cat "$dbg")"
  echo "debugcon: ${body:0:40}..."
  # Exception dump text contains I/S/r/a/2 and would satisfy the tag checks.
  if boot_log_has_exception "$body"; then
    die "CPU exception during boot: ${body:0:240}"
  fi
  case "$body" in *icd*KL*) ;; *) die "expected boot trace icd..KL" ;; esac
  case "$body" in *R*) ;; *) die "expected R (UEFI RAM / MBI parsed)" ;; esac
  case "$body" in *r*M*) ;; *) die "expected r (rust_init / Rust active) before M" ;; esac
  case "$body" in *a*M*) ;; *) die "expected a (Rust GlobalAlloc / kmalloc) before M" ;; esac
  case "$body" in *M*) ;; *) die "expected M (kmain loop entered)" ;; esac
  # Phase2 tags are emitted in order after kmain's M.
  case "$body" in *M*I*) ;; *) die "expected I (Phase2 ioctl→device) after M" ;; esac
  case "$body" in *M*I*B*) ;; *) die "expected B (Phase2 wait-queue wake)" ;; esac
  case "$body" in *M*I*B*S*) ;; *) die "expected S (Phase2 sleep wake)" ;; esac
  case "$body" in *M*I*B*S*2*) ;; *) die "expected 2 (Phase2 self-test passed)" ;; esac
  echo "PASS: debugcon boot trace"
}

_qemu_uefi_pf_restore_cflags() {
  if [[ -n "${_POPCORN_PF_HAD_CFLAGS:-}" ]]; then
    export POPCORN_CFLAGS="$_POPCORN_PF_SAVED_CFLAGS"
  else
    unset POPCORN_CFLAGS
  fi
}

# Put the hardware-safe build back after the POPCORN_TEST_PF kernel. Runs on
# every exit path (failed check, QEMU error, die, set -e via the EXIT trap) so
# target/kernel and the flashable popcorn-uefi.img never keep a kernel that
# page-faults on boot. Pass "exit" when called from the EXIT trap.
_qemu_uefi_pf_restore() {
  local on_exit="${1:-}"
  trap - EXIT
  if [[ -z "$on_exit" && -n "${_POPCORN_PF_PREV_EXIT:-}" ]]; then
    trap -- "$_POPCORN_PF_PREV_EXIT" EXIT
  fi
  _qemu_uefi_pf_restore_cflags
  log INFO "Rebuilding kernel without POPCORN_TEST_PF"
  build_all_uefi
  if [[ -n "$on_exit" && -n "${_POPCORN_PF_PREV_EXIT:-}" ]]; then
    eval "$_POPCORN_PF_PREV_EXIT"
  fi
}

# Rebuild with -DPOPCORN_TEST_PF, boot briefly, expect #PF dump on debugcon/COM1.
qemu_uefi_test_pf() {
  local code dbg serial rc=0
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || { echo "FAIL: edk2-x86_64-code.fd not found" >&2; return 1; }

  log INFO "Building kernel with POPCORN_TEST_PF for exception dump check"
  _POPCORN_PF_HAD_CFLAGS="${POPCORN_CFLAGS+1}"
  _POPCORN_PF_SAVED_CFLAGS="${POPCORN_CFLAGS:-}"
  _POPCORN_PF_PREV_EXIT=""
  if [[ -n "$(trap -p EXIT)" ]]; then
    # Chain any EXIT trap the caller already installed.
    eval "set -- $(trap -p EXIT)"
    _POPCORN_PF_PREV_EXIT="$3"
  fi
  trap '_qemu_uefi_pf_restore exit' EXIT
  export POPCORN_CFLAGS="${POPCORN_CFLAGS:-} -DPOPCORN_TEST_PF"
  build_all_uefi
  _qemu_uefi_pf_restore_cflags

  dbg="$POPCORN_TARGET/pf-debugcon.log"
  serial="$POPCORN_TARGET/pf-serial.log"
  rm -f "$OVMF_VARS"
  ensure_ovmf_vars "$OVMF_VARS"
  rm -f "$dbg" "$serial"
  qemu_kill_guest
  sleep 1

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$serial" \
    -display none -no-reboot \
    -daemonize

  local waited=0
  # Wait for the full dump — '#PF' alone races the kill before CR2= is written.
  while [[ $waited -lt 45 ]]; do
    if grep -q 'CR2=' "$dbg" 2>/dev/null || grep -q 'CR2=' "$serial" 2>/dev/null; then
      break
    fi
    if ! qemu_guest_alive; then
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  qemu_kill_guest

  local body
  body="$(cat "$dbg" 2>/dev/null || true)$(cat "$serial" 2>/dev/null || true)"
  echo "pf log: ${body:0:240}"
  if [[ "$body" != *'#PF'* ]]; then
    echo "FAIL: no #PF dump on serial/debugcon"
    rc=1
  elif [[ "$body" != *CR2=* ]]; then
    echo "FAIL: #PF dump missing CR2="
    rc=1
  elif [[ "$body" != *RIP=* ]]; then
    echo "FAIL: #PF dump missing RIP="
    rc=1
  else
    echo "PASS: #PF serial dump"
  fi

  _qemu_uefi_pf_restore
  return "$rc"
}

qemu_uefi_smoke() {
  echo "== stability (no -no-shutdown, ${POPCORN_QEMU_BOOT_TIMEOUT:-120}s boot timeout) =="
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

# Classify the install selftest from the debugcon text; prints pass, fail or
# pending. Single-letter tags collide (every boot starts with init's 'i', and
# Phase2 emits 'I' after kmain), so only the window between boot_stage('t') and
# boot_stage('e') is read: init_transition_to_console() runs
# rust_test_disk_install there, which emits 'I' (pass) or 'i' (fail). Unique
# INSTOK / INSTFAIL markers take precedence if the kernel emits them.
install_selftest_verdict() {
  local body="$1" window
  if boot_log_has_exception "$body"; then
    echo fail
    return 0
  fi
  case "$body" in
    *INSTFAIL*) echo fail; return 0 ;;
    *INSTOK*) echo pass; return 0 ;;
  esac
  case "$body" in
    *icd*t*e*) ;;
    *) echo pending; return 0 ;;
  esac
  window="${body#*icd}"
  window="${window#*t}"
  window="${window%%e*}"
  case "$window" in
    *i*) echo fail ;;
    *I*) echo pass ;;
    # No marker: the C side refused the selftest (not QEMU) or it never ran.
    *) echo fail ;;
  esac
}

# Boot with POPCORN_TEST_INSTALL kernel (QEMU hypervisor only): runs
# rust_test_disk_install during init onto USB/virtio, never NVMe.
# expects the install 'I' (see install_selftest_verdict), then verifies
# BOOT/KERNEL on the USB data image via mtools.
qemu_uefi_test_install() {
  local code dbg serial boot_timeout
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || { echo "FAIL: edk2-x86_64-code.fd not found" >&2; exit 1; }

  dbg="$POPCORN_TARGET/uefi-install-test.log"
  serial="$POPCORN_TARGET/uefi-install-serial.log"
  boot_timeout="${POPCORN_QEMU_BOOT_TIMEOUT:-180}"

  rm -f "$OVMF_VARS" "$USB_DATA_IMG"
  ensure_ovmf_vars "$OVMF_VARS"
  ensure_raw_img "$USB_DATA_IMG" 64 "USB mass-storage data disk"
  rm -f "$dbg" "$serial"
  qemu_kill_guest
  sleep 1

  qemu_uefi_usb_args "$code" \
    -debugcon "file:$dbg" -global isa-debugcon.iobase=0xe9 \
    -serial "file:$serial" \
    -display none -no-reboot \
    -daemonize

  local waited=0 body="" verdict=""
  while [[ $waited -lt "$boot_timeout" ]]; do
    body="$(cat "$dbg" 2>/dev/null || true)"
    verdict="$(install_selftest_verdict "$body")"
    [[ "$verdict" != pending ]] && break
    if ! qemu_guest_alive; then
      echo "FAIL: QEMU exited early (${waited}s)"
      echo "debugcon: $(cat "$dbg" 2>/dev/null || true)"
      echo "serial: $(cat "$serial" 2>/dev/null || true)"
      return 1
    fi
    sleep 1
    waited=$((waited + 1))
  done

  sleep 2
  qemu_kill_guest
  body="$(cat "$dbg" 2>/dev/null || true)"
  verdict="$(install_selftest_verdict "$body")"
  echo "debugcon: $body"
  if [[ "$verdict" != pass ]]; then
    echo "FAIL: install selftest did not pass (verdict: $verdict)"
    echo "serial: $(cat "$serial" 2>/dev/null || true)"
    return 1
  fi

  # Verify FAT contents on the data image the guest installed onto.
  if have mdir; then
    echo "== mdir BOOT =="
    if ! mdir -i "$USB_DATA_IMG" ::/BOOT; then
      echo "FAIL: mdir ::/BOOT failed on $USB_DATA_IMG"
      return 1
    fi
    if ! mdir -i "$USB_DATA_IMG" ::/BOOT | grep -qi KERNEL; then
      echo "FAIL: KERNEL missing under ::/BOOT"
      return 1
    fi
    # The test loader's UTF-16 banner (bootx64.c, POPCORN_TEST_INSTALL only).
    if have mtype && mtype -i "$USB_DATA_IMG" ::/EFI/BOOT/BOOTX64.EFI 2>/dev/null \
        | LC_ALL=C grep -qaP 't\x00e\x00s\x00t\x00-\x00i\x00n\x00s\x00t\x00a\x00l\x00l\x00 \x00l\x00o\x00a\x00d\x00e\x00r'; then
      echo "FAIL: the test-only loader was installed onto the target"
      return 1
    fi
    if ! mdir -i "$USB_DATA_IMG" ::/EFI/BOOT | grep -qi BOOTX64; then
      echo "FAIL: BOOTX64.EFI missing under ::/EFI/BOOT"
      return 1
    fi
  else
    # Fallback: look for POPCORN OEM + ELF magic somewhere in the image.
    if ! grep -aob $'POPCORN ' "$USB_DATA_IMG" >/dev/null; then
      echo "FAIL: POPCORN OEM BPB not found on data image"
      return 1
    fi
    if ! grep -aob $'\x7fELF' "$USB_DATA_IMG" >/dev/null; then
      echo "FAIL: ELF kernel not found on data image"
      return 1
    fi
  fi
  echo "PASS: disk install selftest (BOOT/KERNEL present on data USB image)"
}

qemu_uefi_run_interactive() {
  local code
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || die "edk2-x86_64-code.fd not found"
  [[ -f "$UEFI_IMG" ]] || die "Missing $UEFI_IMG — run: ./scripts/core.sh img"

  ensure_ovmf_vars "$OVMF_VARS"
  qemu_assert_boot_img
  qemu_assert_guest_file "$OVMF_VARS" "OVMF vars"
  qemu_prepare_scratch_disks
  qemu_uefi_data_disk_args
  # Free the pidfile lock / VNC port held by an earlier Popcorn guest.
  qemu_kill_guest
  log INFO "QEMU UEFI USB boot (interactive window)"
  log INFO "Video: virtio-vga/ramfb (GOP). Serial boot tags still print here."
  log INFO "Guest disks: files under target/ only (host Windows/NVMe never attached)"

  # Video/display args carry no paths, so word-splitting them is safe.
  # shellcheck disable=SC2046
  exec qemu-system-x86_64 \
    -machine q35 -accel kvm -accel tcg -m 4096 -cpu max \
    -drive "if=pflash,format=raw,readonly=on,file=$code" \
    -drive "if=pflash,format=raw,file=$OVMF_VARS" \
    -drive "if=none,id=usbstick,format=raw,file=$UEFI_IMG" \
    -device qemu-xhci,id=xhci \
    -device usb-storage,bus=xhci.0,drive=usbstick,bootindex=0 \
    "${QEMU_DATA_DISK_ARGS[@]}" \
    $(qemu_uefi_video_args) \
    $(qemu_uefi_display_args) \
    -pidfile "$QEMU_PIDFILE" \
    -serial stdio \
    -no-reboot
}


# --- Shell test harness (core.sh test-shell) --------------------------------
# Boots for scripts/lib/shell_test.py. The kernel is built with
# -DPOPCORN_SERIAL_CONSOLE, so COM1 carries the mirrored console plus a
# prompt-ready marker; keys are typed through HMP sendkey on the monitor socket.
# Inputs (env, all set by shell_test.py):
#   SHELL_SERIAL    COM1 capture file
#   SHELL_MON       HMP unix socket path (must be on a native Linux fs)
#   SHELL_DEBUGCON  debugcon (0xE9) capture file
#   SHELL_ALLOW_REBOOT=1  omit -no-reboot (needed for a <REBOOT> step; HMP
#                   system_reset would otherwise power the guest off)
# Daemonized; stop with qemu_kill_guest (pidfile = only this guest).
_shell_boot_common_args() {
  SHELL_QEMU_ARGS=(
    -monitor "unix:${SHELL_MON:?SHELL_MON},server,nowait"
    -debugcon "file:${SHELL_DEBUGCON:?SHELL_DEBUGCON}" -global isa-debugcon.iobase=0xe9
    -display none
  )
  if [[ -n "${SHELL_SERIAL_SOCK:-}" ]]; then
    # COM1 is bidirectional: the harness types bytes into the socket (fast input
    # path of the POPCORN_SERIAL_CONSOLE kernel) and output is still logged.
    rm -f "$SHELL_SERIAL_SOCK"
    SHELL_QEMU_ARGS+=(
      -chardev "socket,id=com1,path=$SHELL_SERIAL_SOCK,server=on,wait=off,logfile=${SHELL_SERIAL:?SHELL_SERIAL}"
      -serial chardev:com1
    )
  else
    SHELL_QEMU_ARGS+=(-serial "file:${SHELL_SERIAL:?SHELL_SERIAL}")
  fi
  [[ "${SHELL_ALLOW_REBOOT:-0}" == 1 ]] || SHELL_QEMU_ARGS+=(-no-reboot)
}

# UEFI: same machine/disks as qemu_uefi_usb_args. The boot stick is opened with
# snapshot=on so a test that wipes or installs over the boot disk never touches
# the image file (writes live in a temp overlay for this QEMU process only).
qemu_uefi_shell_boot() {
  local code
  code="$(find_edk_code || true)"
  [[ -n "$code" ]] || { echo "FAIL: edk2-x86_64-code.fd not found" >&2; return 1; }
  ensure_ovmf_vars "$OVMF_VARS"
  mkdir -p "$(dirname "$SHELL_MON")"
  rm -f "$SHELL_MON"
  _shell_boot_common_args
  QEMU_BOOT_DRIVE_OPTS=",snapshot=on" qemu_uefi_usb_args "$code" \
    "${SHELL_QEMU_ARGS[@]}" -daemonize
}

# GRUB/BIOS: the legacy Multiboot2 ISO (PIT IRQ mode) with the same scratch data
# disks as UEFI (xHCI usb-storage, virtio-blk, NVMe) so both paths see a disk set.
qemu_legacy_shell_boot() {
  local iso="${SHELL_ISO:-$POPCORN_TARGET/UNSAFE/UNSAFE-popcorn-shelltest.iso}"
  qemu_assert_guest_file "$iso" "shell-test ISO"
  mkdir -p "$(dirname "$SHELL_MON")"
  rm -f "$SHELL_MON"
  qemu_kill_guest
  qemu_prepare_scratch_disks
  qemu_uefi_data_disk_args
  _shell_boot_common_args
  qemu-system-x86_64 -machine q35 -accel kvm -accel tcg -m 2048 -cpu max \
    -cdrom "$iso" -boot d \
    -device qemu-xhci,id=xhci \
    "${QEMU_DATA_DISK_ARGS[@]}" \
    -pidfile "$QEMU_PIDFILE" \
    "${SHELL_QEMU_ARGS[@]}" -daemonize
}
