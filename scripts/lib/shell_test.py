#!/usr/bin/env python3
"""Popcorn interactive shell test harness.

Runs inside WSL/Linux. Builds nothing: `./scripts/core.sh test-shell` builds the
-DPOPCORN_SERIAL_CONSOLE kernel/img/ISO under target/UNSAFE/ first, then runs
this script. For each case it boots QEMU through `core.sh _shell-boot-<mode>`
(same machine/disks as qemu_uefi_usb_args for UEFI, the legacy ISO for GRUB),
types every step through HMP `sendkey` on the monitor socket, and reads COM1
(the kernel mirrors the console there and prints `[[POPCORN:PROMPT:<n>]]` after
every drawn shell prompt).

Case file (JSON): a list of
  {"name": str, "boot": "uefi"|"grub"|"both", "why": str,
   "fresh_boot": bool (optional; force a new boot even with --shared),
   "shard_start": bool (optional; the case sets up its own disks, so it and
                  the cases after it can run on a separate guest),
   "quick": bool (optional; part of the --quick smoke tier),
   "steps": [{"send": str, "expect": regex, "reject": regex,
              optional: "timeout": s, "delay_before": s, "enter": bool,
              "allow_exit": bool, "record": {name: regex-with-one-group},
              "input": "ps2" (type this step through HMP sendkey even in
              serial input mode, to exercise the PS/2 scancode path)}]}

`send` syntax:
  * plain text is typed, then Enter is pressed ("" = Enter alone)
  * <ENTER> <LEFT> <RIGHT> <UP> <DOWN> <TAB> <PGUP> <PGDN> <ESC> <BKSP>
    (alias <BACKSPACE>) <HOME> <END> <DEL> are single keys; a send made only of
    such tokens presses just those keys (no Enter). Mixed text + tokens is
    typed in order and then Enter is pressed. "enter" overrides either way.
  * control chars: \\r/\\n = Enter, \\t = Tab, \\b = Backspace, \\x1b = Esc
  * "<REBOOT>" alone = HMP system_reset, then wait for the next first prompt
    (persistence tests; the boot is started without -no-reboot)

A step is done when the next prompt marker arrives (one per Enter typed). In
the Dolphin editor (no shell prompt) and for key-only steps it is done when the
kernel prints `[[POPCORN:IDLE:<n>]]` with n = every key typed so far (the main
loop handled them all and went idle); keys the main loop never sees fall back
to --quiet seconds of COM1 silence. No prompt within the step timeout =
HANG; QEMU gone = DIED (unless the step sets allow_exit and its expect
matched, e.g. `stop`). expect/reject are Python regexes checked against the
step's output (echo included, CRs dropped, backspaces applied, markers
removed). Every step is also checked for a CPU exception dump.

Parallelism: each mode's cases are cut into shards at fresh_boot/shard_start
cases, and up to --jobs shards run at once, each on its own QEMU with private
sockets, pidfile, OVMF vars and scratch disks (target/UNSAFE/UNSAFE-laneN-*).
A boot that misses --boot-timeout keeps its logs and is retried once; it is
listed under "boot anomalies" in the report.

Reports: target/shell-test/report.json, report.txt and per-boot COM1 logs in
target/shell-test/logs/.
"""

from __future__ import annotations

import argparse
import codecs
import json
import os
import re
import shutil
import socket
import subprocess
import threading
from concurrent.futures import ThreadPoolExecutor
import sys
import time
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CORE = ROOT / "scripts" / "core.sh"
TARGET = ROOT / "target"
RUNTIME = Path(os.environ.get("RUNTIME_DIR", "/tmp/popcorn-qemu"))

MARKER_RE = re.compile(r"(?:\r?\n)?\[\[POPCORN:PROMPT:(\d+)\]\]\r?\n")
# Main loop idle after handling keys; the number is keys handled since boot (kernel.c).
IDLE_RE = re.compile(r"(?:\r?\n)?\[\[POPCORN:IDLE:(\d+)\]\]\r?\n")
FAULT_RE = re.compile(r"\*\*\* CPU EXCEPTION|\*\*\* halt \*\*\*")
EDITOR_OPEN_RE = re.compile(r"ESC for commands \(w,q,wq,q!\)")
EDITOR_CLOSE_RE = re.compile(r"Dolphin editor closed|Dolphin closed \(changes discarded\)")

SPECIAL_KEYS = {
    "<ENTER>": "ret", "<RET>": "ret",
    "<LEFT>": "left", "<RIGHT>": "right", "<UP>": "up", "<DOWN>": "down",
    "<TAB>": "tab", "<PGUP>": "pgup", "<PGDN>": "pgdn", "<ESC>": "esc",
    "<BKSP>": "backspace", "<BACKSPACE>": "backspace",
    "<HOME>": "home", "<END>": "end", "<DEL>": "delete",
}
# Keys the POPCORN_SERIAL_CONSOLE kernel accepts as COM1 bytes (kernel.c).
SERIAL_SPECIAL = {"ret": b"\r", "backspace": b"\x7f", "esc": b"\x1b", "tab": b"\t"}
TOKEN_RE = re.compile("(" + "|".join(re.escape(k) for k in SPECIAL_KEYS) + ")")

# US layout: unshifted punctuation and their shifted counterparts.
PLAIN = {
    " ": "spc", "-": "minus", "=": "equal", "[": "bracket_left",
    "]": "bracket_right", "\\": "backslash", ";": "semicolon",
    "'": "apostrophe", "`": "grave_accent", ",": "comma", ".": "dot",
    "/": "slash", "\r": "ret", "\n": "ret", "\t": "tab", "\b": "backspace",
    "\x7f": "backspace", "\x1b": "esc",
}
SHIFTED = {
    "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7",
    "*": "8", "(": "9", ")": "0", "_": "minus", "+": "equal",
    "{": "bracket_left", "}": "bracket_right", "|": "backslash",
    ":": "semicolon", '"': "apostrophe", "~": "grave_accent", "<": "comma",
    ">": "dot", "?": "slash",
}


_tls = threading.local()
_print_lock = threading.Lock()


def say(msg: str) -> None:
    """print() with the current lane's tag, so parallel lanes stay readable."""
    tag = getattr(_tls, "tag", "")
    with _print_lock:
        print(f"{tag}{msg}", flush=True)


def _drain_socket(sock: socket.socket) -> None:
    try:
        while sock.recv(65536):
            pass
    except OSError:
        pass


def _serial_bytes() -> dict[str, bytes]:
    m = dict(SERIAL_SPECIAL)
    for c in map(chr, range(32, 127)):
        try:
            m.setdefault(char_key(c), c.encode())
        except ValueError:
            pass
    return m


def char_key(ch: str) -> str:
    if "a" <= ch <= "z" or "0" <= ch <= "9":
        return ch
    if "A" <= ch <= "Z":
        return "shift-" + ch.lower()
    if ch in PLAIN:
        return PLAIN[ch]
    if ch in SHIFTED:
        return "shift-" + SHIFTED[ch]
    raise ValueError(f"no key mapping for {ch!r}")


def parse_send(send: str, enter: bool | None) -> list[str]:
    """Turn a step's `send` into QEMU key names (Enter appended per the rules)."""
    keys: list[str] = []
    literal = False
    for part in TOKEN_RE.split(send):
        if not part:
            continue
        if part in SPECIAL_KEYS:
            keys.append(SPECIAL_KEYS[part])
        else:
            literal = True
            keys.extend(char_key(c) for c in part)
    if enter is None:
        enter = literal or not keys  # "" = Enter alone; key-only steps: no Enter
    if enter:
        keys.append("ret")
    return keys


SERIAL_BYTES: dict[str, bytes] = {}


def normalize(raw: str) -> str:
    """COM1 text as a reader would see it: markers gone, CRs dropped, BS applied."""
    s = IDLE_RE.sub("", MARKER_RE.sub("", raw)).replace("\r", "")
    out: list[str] = []
    for ch in s:
        if ch == "\b":
            if out and out[-1] != "\n":
                out.pop()
        else:
            out.append(ch)
    return "".join(out)


class HarnessError(Exception):
    pass


# First bytes of the modern-only virtio-blk test disk (see Lane.reset_disks).
MODERN_SIGNATURE = b"POPCORN-VIRTIO-M0"


class BootTimeout(HarnessError):
    pass


class Lane:
    """Private paths for one QEMU guest, so several guests can run at once."""

    def __init__(self, lane_id: int):
        self.id = lane_id
        self.dir = RUNTIME / f"lane{lane_id}"
        self.pidfile = self.dir / "popcorn-qemu.pid"
        u = TARGET / "UNSAFE"
        self.disks = [u / f"UNSAFE-lane{lane_id}-qemu-virtio.img",
                      u / f"UNSAFE-lane{lane_id}-qemu-usb-data.img",
                      u / f"UNSAFE-lane{lane_id}-qemu-nvme.img",
                      u / f"UNSAFE-lane{lane_id}-qemu-modern.img"]
        self.ovmf_vars = u / f"UNSAFE-lane{lane_id}-shell-ovmf-vars.fd"

    def env(self) -> dict:
        env = dict(os.environ)
        env.update({
            "RUNTIME_DIR": str(self.dir),
            "QEMU_PIDFILE": str(self.pidfile),
            "DATA_IMG": str(self.disks[0]),
            "USB_DATA_IMG": str(self.disks[1]),
            "INTERNAL_IMG": str(self.disks[2]),
            "MODERN_IMG": str(self.disks[3]),
            "OVMF_VARS": str(self.ovmf_vars),
        })
        return env

    def reset_disks(self) -> None:
        for path in self.disks + [self.ovmf_vars]:
            path = path.resolve()
            # Only ever delete our own QEMU scratch files.
            if "/target/UNSAFE/" not in str(path) or not path.name.startswith("UNSAFE-lane"):
                raise HarnessError(f"refusing to reset non-scratch disk {path}")
            if path.exists():
                path.unlink()
        # Modern-only virtio-blk for `virtio -probe`: 8 MiB with a known first sector.
        modern = self.disks[3]
        with open(modern, "wb") as fh:
            fh.write(MODERN_SIGNATURE.ljust(512, b"\0"))
            fh.truncate(8 << 20)

    def kill(self) -> None:
        subprocess.run([str(CORE), "_shell-kill"], cwd=ROOT, env=self.env(),
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class Monitor:
    """HMP over the QEMU unix monitor socket."""

    def __init__(self, path: Path, timeout: float = 30.0):
        deadline = time.time() + timeout
        last = None
        while True:
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(str(path))
                break
            except OSError as e:
                last = e
                s.close()
                if time.time() > deadline:
                    raise HarnessError(f"cannot connect HMP socket {path}: {last}")
                time.sleep(0.2)
        self.sock = s
        self.sock.settimeout(10)
        self._read_prompt()

    def _read_prompt(self) -> str:
        buf = b""
        deadline = time.time() + 15
        while not buf.rstrip().endswith(b"(qemu)"):
            if time.time() > deadline:
                raise HarnessError(f"HMP prompt timeout; got {buf[-200:]!r}")
            try:
                chunk = self.sock.recv(4096)
            except socket.timeout:
                continue
            if not chunk:
                raise HarnessError("HMP socket closed")
            buf += chunk
        return buf.decode("latin-1", "replace")

    def cmd(self, line: str) -> str:
        self.sock.sendall(line.encode() + b"\n")
        return self._read_prompt()

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


class Guest:
    """One QEMU boot: COM1 capture file + HMP monitor."""

    def __init__(self, mode: str, args, allow_reboot: bool, log_name: str, lane: Lane):
        self.mode = mode
        self.args = args
        self.allow_reboot = allow_reboot
        self.lane = lane
        self.serial = lane.dir / "shell-serial.log"
        self.debugcon = lane.dir / "shell-debugcon.log"
        self.mon_path = lane.dir / "shell-mon.sock"
        # Keys typed since boot vs. the kernel's latest IDLE count (kernel.c).
        self.keys_sent = 0
        self.idle_seen = 0
        self.idle_scan = 0
        self.send_mark = 0
        self.boot_anomalies: list[str] = []
        self.log_name = log_name
        self.raw = ""
        self.cursor = 0
        self._fh = None
        self.mon: Monitor | None = None
        self.last_seq = 0
        self.editor = False
        self.hold_ms = args.hold_ms
        self.serial_sock_path = lane.dir / "shell-com1.sock"
        self.com1: socket.socket | None = None
        self.input_mode = args.input  # "serial" (COM1 bytes) or "ps2" (HMP sendkey)
        # Dolphin: ESC opens the ':' prompt, which waits for the PS/2 ESC release
        # (0x81) and then reads only the scancode queue (dolphin.rs
        # handle_command_mode), never COM1. So ESC and everything up to the next
        # Enter go through sendkey (both boot paths run on IRQ0 now).
        self.ps2_latch = False

    # -- lifecycle -----------------------------------------------------------
    def start(self, retry: bool = True) -> str:
        self.lane.dir.mkdir(parents=True, exist_ok=True)
        self.raw, self.cursor, self.idle_scan = "", 0, 0
        for p in (self.serial, self.debugcon):
            try:
                p.unlink()
            except FileNotFoundError:
                pass
        env = self.lane.env()
        env.update({
            "SHELL_SERIAL": str(self.serial),
            "SHELL_DEBUGCON": str(self.debugcon),
            "SHELL_MON": str(self.mon_path),
            "SHELL_SERIAL_SOCK": str(self.serial_sock_path) if self.args.input != "ps2" else "",
            "SHELL_ALLOW_REBOOT": "1" if self.allow_reboot else "0",
        })
        sub = "_shell-boot-uefi" if self.mode == "uefi" else "_shell-boot-grub"
        r = subprocess.run([str(CORE), sub], cwd=ROOT, env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if r.returncode != 0:
            raise HarnessError(f"{sub} failed (rc={r.returncode}):\n{r.stdout[-2000:]}")
        deadline = time.time() + 20
        while not self.serial.exists():
            if time.time() > deadline:
                raise HarnessError("QEMU did not create the COM1 capture file")
            time.sleep(0.1)
        self._fh = open(self.serial, "rb")
        # Kernel output is UTF-8 (e.g. em dashes); decode incrementally so a
        # multi-byte char split across reads is not mangled into latin-1 junk.
        self._dec = codecs.getincrementaldecoder("utf-8")("replace")
        self.mon = Monitor(self.mon_path)
        if self.args.input != "ps2":
            deadline = time.time() + 20
            while True:
                try:
                    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    s.connect(str(self.serial_sock_path))
                    self.com1 = s
                    # QEMU also sends COM1 output to the socket client and blocks the
                    # guest UART when that buffer fills; drain it (the logfile has it).
                    threading.Thread(target=_drain_socket, args=(s,), daemon=True).start()
                    break
                except OSError as e:
                    s.close()
                    if time.time() > deadline:
                        raise HarnessError(f"cannot connect COM1 socket: {e}")
                    time.sleep(0.1)
        try:
            return self.wait_boot()
        except BootTimeout as e:
            if not retry:
                raise
            # Keep the evidence (the intermittent UEFI loader hang), then boot again.
            self.note_boot_hang(str(e))
            self.stop(save_logs=False)
            return self.start(retry=False)

    def note_boot_hang(self, why: str) -> None:
        logs = Path(self.args.out_dir) / "logs"
        logs.mkdir(parents=True, exist_ok=True)
        tag = f"{self.log_name}-boot-hang-{len(self.boot_anomalies) + 1}"
        try:
            (logs / f"{tag}-serial.log").write_text(self.raw, encoding="utf-8")
            shutil.copyfile(self.debugcon, logs / f"{tag}-debugcon.log")
        except OSError:
            pass
        first = why.splitlines()[0] if why else "boot timeout"
        self.boot_anomalies.append(f"{first} (logs: logs/{tag}-*.log; retried)")
        say(f"    [BOOT HANG] {self.log_name}: {first}; logs saved as {tag}, retrying")

    def wait_boot(self) -> str:
        """Wait for the first prompt marker of a (re)boot, then let boot output settle."""
        start = self.cursor
        deadline = time.time() + self.args.boot_timeout
        while True:
            self.poll()
            m = MARKER_RE.search(self.raw, self.cursor)
            if m:
                self.last_seq = int(m.group(1))
                self.cursor = m.end()
                break
            if not self.alive():
                raise HarnessError("QEMU exited during boot:\n" + normalize(self.raw[start:])[-1500:])
            if time.time() > deadline:
                raise BootTimeout(f"no shell prompt within {self.args.boot_timeout}s of boot:\n"
                                  + normalize(self.raw[start:])[-1500:])
            time.sleep(0.05)
        # Phase 2 selftest still writes boot tags to COM1 after the first prompt.
        self.wait_quiet(1.0, 20)
        boot_text = self.raw[start:]
        self.cursor = len(self.raw)
        self.editor = False
        # The kernel's key counter restarts with every boot.
        self.keys_sent = self.idle_seen = 0
        self.idle_scan = len(self.raw)
        return boot_text

    def stop(self, save_logs: bool = True) -> None:
        if self.com1:
            try:
                self.com1.close()
            except OSError:
                pass
            self.com1 = None
        if self.mon:
            self.mon.close()
            self.mon = None
        self.lane.kill()
        self.poll()
        if self._fh:
            self._fh.close()
            self._fh = None
        if not save_logs:
            return
        logs = Path(self.args.out_dir) / "logs"
        logs.mkdir(parents=True, exist_ok=True)
        try:
            shutil.copyfile(self.serial, logs / f"{self.log_name}-serial.log")
            shutil.copyfile(self.debugcon, logs / f"{self.log_name}-debugcon.log")
        except OSError:
            pass

    def alive(self) -> bool:
        try:
            pid = int(self.lane.pidfile.read_text().strip())
            comm = Path(f"/proc/{pid}/comm").read_text()
            return "qemu" in comm
        except (OSError, ValueError):
            return False

    # -- I/O -----------------------------------------------------------------
    def poll(self) -> int:
        if not self._fh:
            return 0
        data = self._fh.read()
        if data:
            self.raw += self._dec.decode(data)
            for m in IDLE_RE.finditer(self.raw, self.idle_scan):
                self.idle_seen = max(self.idle_seen, int(m.group(1)))
                self.idle_scan = m.end()
        return len(data) if data else 0

    def wait_idle(self, quiet: float, limit: float) -> bool:
        """Wait until the kernel reports it has handled every key sent so far.
        Falls back to `quiet` seconds of COM1 silence for keys the main loop
        never sees (GRUB Dolphin's ':' prompt reads the keyboard itself), and
        then trusts the kernel's count from there on."""
        end = time.time() + limit
        last = time.time()
        while True:
            if self.poll():
                last = time.time()
            if self.idle_seen >= self.keys_sent:
                return True
            now = time.time()
            if now - last >= quiet or now >= end:
                self.keys_sent = self.idle_seen
                return False
            time.sleep(0.005)

    def wait_quiet(self, quiet: float, limit: float) -> None:
        end = time.time() + limit
        last = time.time()
        while time.time() < end:
            if self.poll():
                last = time.time()
            elif time.time() - last >= quiet:
                return
            time.sleep(0.02)

    def sendkey(self, key: str) -> None:
        assert self.mon
        self.mark_send()
        self.mon.cmd(f"sendkey {key} {self.hold_ms}")

    def mark_send(self) -> None:
        """Remember where COM1 output stood when keys went out: only an IDLE
        marker after this point can finish the step (an older one is stale)."""
        self.poll()
        self.send_mark = len(self.raw)

    def type_keys(self, keys: list[str], force_ps2: bool = False) -> None:
        """Type keys. In serial mode, runs of plain characters / Enter / Backspace /
        Esc / Tab go out as one COM1 burst (QEMU's UART only accepts bytes as the
        FIFO drains, so nothing is dropped); other keys use PS/2 sendkey."""
        if self.com1 is None or self.input_mode == "ps2" or force_ps2:
            return self.type_keys_ps2(keys)
        burst = b""
        for k in keys:
            if k == "esc" or self.ps2_latch:
                if burst:
                    self.mark_send()
                    self.com1.sendall(burst)
                    self.keys_sent += len(burst)
                    burst = b""
                    self.wait_idle(0.3, 5.0)
                self.type_keys_ps2([k])
                self.ps2_latch = k != "ret"
                continue
            b = SERIAL_BYTES.get(k)
            if b is not None:
                burst += b
                continue
            if burst:
                self.mark_send()
                self.com1.sendall(burst)
                self.keys_sent += len(burst)
                burst = b""
                self.wait_idle(0.3, 5.0)
            self.type_keys_ps2([k])
        if burst:
            self.mark_send()
            self.com1.sendall(burst)
            self.keys_sent += len(burst)

    def type_keys_ps2(self, keys: list[str]) -> None:
        """Type keys through PS/2, pacing on the guest's echo.

        QEMU's input queue drops events past ~50 entries, and sendkey's delayed
        key releases back up when the guest is busy (a Dolphin redraw per key
        under TCG). A dropped Shift release then corrupts every later key. So
        after each key wait for its echo to start and then for COM1 to go
        quiet (the redraw finished) before sending the next one.
        """
        grow_wait = 0.3
        settle = self.args.key_settle
        for k in keys:
            before = os.path.getsize(self.serial)
            self.sendkey(k)
            self.keys_sent += 1
            time.sleep(self.hold_ms / 1000.0 + 0.004)
            if k == "ret":
                continue  # command output is awaited per step, not per key
            # The kernel announces when it has handled this key (and redrawn).
            if self.wait_idle(0.3, 5.0):
                continue
            end = time.time() + grow_wait
            grew = False
            while time.time() < end:
                if os.path.getsize(self.serial) > before:
                    grew = True
                    break
                time.sleep(0.004)
            # No echo (line full, TAB with no match, scroll at top): stop waiting long.
            grow_wait = 0.3 if grew else 0.04
            if grew:
                size = os.path.getsize(self.serial)
                quiet_since = time.time()
                hard = time.time() + 5.0
                while time.time() < hard:
                    time.sleep(0.005)
                    n = os.path.getsize(self.serial)
                    if n != size:
                        size, quiet_since = n, time.time()
                    elif time.time() - quiet_since >= settle:
                        break

    # -- steps ---------------------------------------------------------------
    def run_keys(self, keys: list[str], timeout: float, settle_quiet: bool,
                 allow_exit: bool, force_ps2: bool = False) -> tuple[str, str]:
        """Send keys, wait for completion. Returns (raw output, how)."""
        start = self.cursor
        self.type_keys(keys, force_ps2)
        need = sum(1 for k in keys if k == "ret") if not self.editor else 1
        need = max(need, 1)
        t_sent = time.time()
        last_growth = t_sent
        found = 0
        scan = start
        quiet = self.args.quiet
        while True:
            if self.poll():
                last_growth = time.time()
            while found < need:
                m = MARKER_RE.search(self.raw, scan)
                if not m:
                    break
                seq = int(m.group(1))
                if seq <= self.last_seq:
                    out = self.raw[start:m.start()]
                    self.cursor = m.end()
                    self.last_seq = seq
                    return out, "rebooted"
                self.last_seq = seq
                scan = m.end()
                found += 1
                if self.editor:
                    break
            now = time.time()
            if found >= need or (self.editor and found):
                # A marker ends a shell step; anything after it belongs to the next one.
                out = self.raw[start:scan]
                self.cursor = scan
                return out, "prompt"
            if (settle_quiet or self.editor) and self.idle_seen >= self.keys_sent \
                    and self.idle_scan > max(start, self.send_mark):
                # Every key handled and redrawn (IDLE marker): the step is done.
                out = self.raw[start:self.idle_scan]
                self.cursor = self.idle_scan
                return out, "idle"
            if (settle_quiet or self.editor) and now - last_growth >= quiet:
                self.keys_sent = self.idle_seen
                out = self.raw[start:]
                self.cursor = len(self.raw)
                return out, "quiet"
            if not self.alive():
                time.sleep(0.2)
                self.poll()
                out = self.raw[start:]
                self.cursor = len(self.raw)
                return out, "exit"
            if now - t_sent > timeout:
                out = self.raw[start:]
                self.cursor = len(self.raw)
                return out, "timeout"
            time.sleep(0.02)

    def reboot(self) -> str:
        assert self.mon
        self.mon.cmd("system_reset")
        self.last_seq = 0
        try:
            return self.wait_boot()
        except BootTimeout as e:
            self.note_boot_hang(str(e))
            self.mon.cmd("system_reset")
            self.last_seq = 0
            return self.wait_boot()

    def update_editor_state(self, out: str) -> None:
        events = [(m.start(), True) for m in EDITOR_OPEN_RE.finditer(out)]
        events += [(m.start(), False) for m in EDITOR_CLOSE_RE.finditer(out)]
        if events:
            self.editor = sorted(events)[-1][1]


def check_step(step: dict, out_norm: str, out_raw: str, how: str) -> tuple[str, str]:
    """Return (status, reason). status in pass|fail|hang|died|rebooted."""
    expect = step.get("expect") or ""
    reject = step.get("reject") or ""
    exp_ok = (not expect) or re.search(expect, out_norm) is not None
    rej_m = re.search(reject, out_norm) if reject else None
    if FAULT_RE.search(out_raw):
        return "fail", "CPU exception dump on COM1"
    if how == "rebooted":
        return "rebooted", "guest rebooted unexpectedly (prompt counter restarted)"
    if how == "exit":
        if step.get("allow_exit") and exp_ok and not rej_m:
            return "pass", "guest exited (allowed)"
        return "died", "QEMU exited" + ("" if exp_ok else "; expect not matched")
    if how == "timeout":
        if step.get("allow_exit") and exp_ok and not rej_m:
            return "pass", "no prompt after allowed-exit step (halted)"
        return "hang", ("no prompt within timeout"
                        + ("" if exp_ok else "; expect not matched")
                        + (f"; reject matched {rej_m.group(0)!r}" if rej_m else ""))
    if not exp_ok:
        return "fail", "expect not matched"
    if rej_m:
        return "fail", f"reject matched {rej_m.group(0)!r}"
    return "pass", ""


def run_case(g: Guest, case: dict, args) -> dict:
    res = {"name": case["name"], "why": case.get("why", ""), "steps": [], "status": "pass"}
    t_case = time.time()
    for i, step in enumerate(case["steps"]):
        send = step.get("send", "")
        rec = {"index": i, "send": send, "expect": step.get("expect", ""),
               "reject": step.get("reject", "")}
        if step.get("delay_before"):
            time.sleep(float(step["delay_before"]))
        t0 = time.time()
        rec["t_start"] = round(t0 - t_case, 3)
        try:
            if send == "<REBOOT>":
                out_raw = g.reboot()
                how = "prompt"
            else:
                keys = parse_send(send, step.get("enter"))
                settle_quiet = "ret" not in keys
                out_raw, how = g.run_keys(keys, float(step.get("timeout", args.step_timeout)),
                                          settle_quiet, bool(step.get("allow_exit")),
                                          step.get("input") == "ps2")
        except HarnessError as e:
            out_raw, how = str(e), "exit"
        out = normalize(out_raw)
        status, reason = check_step(step, out, out_raw, how)
        g.update_editor_state(out)
        rec.update({"status": status, "reason": reason, "settled": how,
                    "elapsed": round(time.time() - t0, 3),
                    "host_time": round(time.time(), 3), "output": out[-6000:]})
        if step.get("record"):
            rec["record"] = {}
            for name, rx in step["record"].items():
                m = re.search(rx, out)
                rec["record"][name] = m.group(1) if m and m.groups() else (m.group(0) if m else None)
        res["steps"].append(rec)
        mark = "ok " if status == "pass" else status.upper()
        if args.verbose or status != "pass":
            say(f"    [{mark}] {case['name']} step {i}: {send[:60]!r} ({how}, {rec['elapsed']}s) {reason}")
        if status != "pass":
            res["status"] = status
            if status in ("hang", "died", "rebooted") or args.stop_on_fail:
                res["aborted_at"] = i
                break
    res["elapsed"] = round(time.time() - t_case, 1)
    return res


def plan_shards(cases: list[dict], mode: str, prev: dict) -> list[dict]:
    """Split one mode's cases into shards that can run on separate guests.

    A shard starts at a case marked fresh_boot or shard_start: those cases
    expect a new boot and (shard_start) fresh scratch disks, so nothing before
    them is needed. Cases in between share one boot and disk state in order.
    """
    shards: list[dict] = []
    for idx, c in cases:
        if c["boot"] not in (mode, "both"):
            continue
        if not shards or c.get("fresh_boot") or c.get("shard_start"):
            shards.append({"mode": mode, "cases": [], "est": 0.0})
        shards[-1]["cases"].append((idx, c))
        shards[-1]["est"] += prev.get((mode, c["name"]), 2.0 + 0.5 * len(c["steps"]))
    return shards


def previous_timings(out_dir: Path) -> dict:
    try:
        doc = json.loads((out_dir / "report.json").read_text(encoding="utf-8"))
        return {(r["mode"], r["name"]): float(r.get("elapsed", 0)) + 0.0
                for r in doc.get("results", [])}
    except (OSError, ValueError, KeyError):
        return {}


def run_shard(shard: dict, lane: Lane, args) -> list[dict]:
    mode = shard["mode"]
    _tls.tag = f"[{mode}/L{lane.id}] "
    results: list[dict] = []
    if not args.keep_disks:
        lane.reset_disks()
    cases = shard["cases"]
    g: Guest | None = None
    try:
        for n, (idx, case) in enumerate(cases):
            needs_reboot = any(s.get("send") == "<REBOOT>" for s in case["steps"])
            fresh = (not args.shared) or g is None or (n > 0 and case.get("fresh_boot")) \
                or (needs_reboot and not g.allow_reboot)
            if fresh and g is not None:
                g.stop()
                g = None
            say(f"-> {case['name']}")
            res = {"mode": mode, "name": case["name"], "order": idx}
            try:
                if g is None:
                    allow = needs_reboot or (args.shared and any(
                        s.get("send") == "<REBOOT>" for _, c in cases[n:] for s in c["steps"]))
                    g = Guest(mode, args, allow, f"{mode}-{idx:02d}-{case['name']}", lane)
                    g.start()
                before = len(g.boot_anomalies)
                res.update(run_case(g, case, args))
                res["mode"] = mode
                if len(g.boot_anomalies) > before:
                    res["boot_anomalies"] = g.boot_anomalies[before:]
            except HarnessError as e:
                res.update({"status": "error", "error": str(e), "steps": [],
                            "why": case.get("why", "")})
                if g is not None and g.boot_anomalies:
                    res["boot_anomalies"] = list(g.boot_anomalies)
                say(f"    [ERROR] {case['name']}: {e}")
            results.append(res)
            say(f"   {res['status'].upper()} {case['name']} ({res.get('elapsed', 0)}s)")
            if g is not None and (res["status"] in ("hang", "died", "rebooted", "error")
                                  or not g.alive()):
                g.stop()
                g = None
    finally:
        if g is not None:
            g.stop()
    if args.fsck:
        results.extend(fsck_lane_disks(shard, lane))
    return results


def fsck_lane_disks(shard: dict, lane: Lane) -> list[dict]:
    """Check every FAT32 scratch disk this shard left behind with Linux's own
    fsck.fat (read-only), so FAT damage from Popcorn fails the run even when
    the shell output looked fine."""
    fsck = shutil.which("fsck.fat") or shutil.which("dosfsck")
    if not fsck:
        return []
    out = []
    first = shard["cases"][0][1]["name"]
    for disk in lane.disks:
        try:
            with open(disk, "rb") as f:
                sec = f.read(512)
        except OSError:
            continue
        if len(sec) < 512 or sec[510:512] != b"\x55\xaa" or sec[82:87] != b"FAT32":
            continue
        r = subprocess.run([fsck, "-n", str(disk)], stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True)
        lines = [ln for ln in r.stdout.splitlines() if ln.strip()]
        # A clean volume prints only the version banner and the summary line.
        clean = r.returncode == 0 and len(lines) <= 2
        name = f"fsck_{disk.name.split('-qemu-')[-1].removesuffix('.img')}_after_{first}"
        res = {"mode": shard["mode"], "name": name, "order": shard["cases"][-1][0] + 0.5,
               "why": "fsck.fat -n on a scratch disk the shard wrote", "steps": [],
               "status": "pass" if clean else "fail", "elapsed": 0}
        if not clean:
            res["error"] = f"fsck.fat rc={r.returncode}:\n" + r.stdout[-1500:]
        say(f"   {res['status'].upper()} {name}")
        out.append(res)
    return out


def load_cases(path: Path) -> list[dict]:
    cases = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(cases, list):
        raise HarnessError("cases file must be a JSON list")
    seen = set()
    for c in cases:
        for k in ("name", "boot", "steps"):
            if k not in c:
                raise HarnessError(f"case missing {k!r}: {str(c)[:120]}")
        if c["name"] in seen:
            raise HarnessError(f"duplicate case name {c['name']}")
        seen.add(c["name"])
        if c["boot"] not in ("uefi", "grub", "both"):
            raise HarnessError(f"{c['name']}: bad boot {c['boot']!r}")
        for s in c["steps"]:
            for k in ("expect", "reject"):
                if s.get(k):
                    re.compile(s[k])
            if s.get("input") not in (None, "ps2", "serial"):
                raise HarnessError(f"{c['name']}: bad step input {s.get('input')!r}")
            if s.get("send", "") != "<REBOOT>":
                parse_send(s.get("send", ""), s.get("enter"))
    return cases


def write_reports(results: list[dict], args, started: float) -> tuple[int, int]:
    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    total = len(results)
    failed = sum(1 for r in results if r["status"] != "pass")
    doc = {"started": datetime.fromtimestamp(started).isoformat(timespec="seconds"),
           "elapsed_s": round(time.time() - started, 1), "cases_file": str(args.cases),
           "total": total, "failed": failed, "results": results}
    (out / "report.json").write_text(json.dumps(doc, indent=1), encoding="utf-8")
    lines = [f"Popcorn shell test report  {doc['started']}  ({doc['elapsed_s']}s)",
             f"cases file: {args.cases}", ""]
    for mode in ("uefi", "grub"):
        rs = [r for r in results if r["mode"] == mode]
        if not rs:
            continue
        ok = sum(1 for r in rs if r["status"] == "pass")
        lines.append(f"== {mode}: {ok}/{len(rs)} cases passed")
        for r in rs:
            nsteps = len(r["steps"])
            okst = sum(1 for s in r["steps"] if s["status"] == "pass")
            lines.append(f"  {r['status'].upper():9} {r['name']}  ({okst}/{nsteps} steps, {r.get('elapsed', 0)}s)")
        lines.append("")
    bad = [r for r in results if r["status"] != "pass"]
    if bad:
        lines.append("== failure details")
        for r in bad:
            lines.append(f"-- [{r['mode']}] {r['name']}: {r['status']}")
            if r.get("error"):
                lines.append(f"   harness: {r['error']}")
            for s in r["steps"]:
                if s["status"] == "pass":
                    continue
                lines.append(f"   step {s['index']} send={s['send']!r} -> {s['status']} ({s['reason']}; settled={s['settled']})")
                if s.get("expect"):
                    lines.append(f"     expect: {s['expect']}")
                if s.get("reject"):
                    lines.append(f"     reject: {s['reject']}")
                tail = s["output"][-1500:].replace("\n", "\n       | ")
                lines.append(f"     output (tail):\n       | {tail}")
            lines.append("")
    anomalies = [(r, a) for r in results for a in r.get("boot_anomalies", [])]
    if anomalies:
        lines.append("== boot anomalies (guest did not reach the shell; retried once)")
        for r, a in anomalies:
            lines.append(f"  [{r['mode']}] {r['name']}: {a}")
        lines.append("")
    lines.append(f"TOTAL: {total - failed}/{total} passed, {failed} failed"
                 + (f", {len(anomalies)} boot anomalies" if anomalies else ""))
    (out / "report.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return total, failed


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cases", default=str(ROOT / "scripts" / "tests" / "shell_cases.json"))
    ap.add_argument("--mode", choices=("uefi", "grub", "both"), default="both")
    ap.add_argument("--only", help="regex: run cases whose name matches")
    ap.add_argument("--exclude", help="regex: skip cases whose name matches")
    ap.add_argument("--shared", action=argparse.BooleanOptionalAction, default=True,
                    help="run consecutive cases on one boot (default; reboots after a hang/death or on "
                         "fresh_boot). --no-shared boots fresh for every case.")
    ap.add_argument("--keep-disks", action="store_true",
                    help="do not reset the scratch disks before each mode")
    ap.add_argument("--step-timeout", type=float, default=45.0)
    ap.add_argument("--boot-timeout", type=float, default=60.0,
                    help="seconds to reach the first prompt; on timeout the logs are kept and the "
                         "boot is retried once (reported under boot anomalies)")
    ap.add_argument("-j", "--jobs", type=int, default=min(6, max(1, (os.cpu_count() or 2) // 2)),
                    help="QEMU guests to run at once (shards of the case list)")
    ap.add_argument("--fsck", action=argparse.BooleanOptionalAction, default=True,
                    help="after each shard, check FAT32 scratch disks with fsck.fat -n (default on)")
    ap.add_argument("--quick", action="store_true",
                    help="run only cases marked \"quick\": true (fast smoke tier)")
    ap.add_argument("--quiet", type=float, default=1.0,
                    help="seconds of COM1 silence that settle an editor/key-only step")
    ap.add_argument("--input", choices=("serial", "ps2"), default="serial",
                    help="serial: type via COM1 bytes (fast; PS/2 only for arrows etc.); "
                         "ps2: every key through HMP sendkey (exercises the keyboard driver)")
    ap.add_argument("--hold-ms", type=int, default=10, help="sendkey hold time")
    ap.add_argument("--key-settle", type=float, default=0.03,
                    help="COM1 quiet time after a key's echo before the next key")
    ap.add_argument("--stop-on-fail", action="store_true", help="abort a case at its first failing step")
    ap.add_argument("--out-dir", default=str(TARGET / "shell-test"))
    ap.add_argument("--list", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()
    SERIAL_BYTES.update(_serial_bytes())

    try:
        cases = load_cases(Path(args.cases))
    except (HarnessError, ValueError, re.error) as e:
        print(f"shell_test: bad cases file: {e}", file=sys.stderr)
        return 2
    if args.quick:
        cases = [c for c in cases if c.get("quick")]
    if args.only:
        cases = [c for c in cases if re.search(args.only, c["name"])]
    if args.exclude:
        cases = [c for c in cases if not re.search(args.exclude, c["name"])]
    modes = ["uefi", "grub"] if args.mode == "both" else [args.mode]
    if args.list:
        for c in cases:
            print(f"{c['boot']:5} {len(c['steps']):4} steps  {c['name']}")
        return 0

    started = time.time()
    results: list[dict] = []
    logs = Path(args.out_dir) / "logs"
    if logs.exists():
        shutil.rmtree(logs, ignore_errors=True)
    prev = previous_timings(Path(args.out_dir))
    indexed = list(enumerate(cases))
    shards = [sh for mode in modes for sh in plan_shards(indexed, mode, prev)]
    # Longest first, so the slowest shard is not the one left running alone.
    shards.sort(key=lambda sh: -sh["est"])
    jobs = max(1, min(args.jobs, len(shards)))
    print(f"== {sum(len(sh['cases']) for sh in shards)} case runs in {len(shards)} shards "
          f"on {jobs} parallel guests ({', '.join(modes)})", flush=True)
    free = list(range(jobs))
    free_lock = threading.Lock()

    def worker(sh: dict) -> list[dict]:
        with free_lock:
            lane = Lane(free.pop())
        try:
            return run_shard(sh, lane, args)
        finally:
            with free_lock:
                free.append(lane.id)

    pool = ThreadPoolExecutor(max_workers=jobs)
    try:
        for part in pool.map(worker, shards):
            results.extend(part)
        pool.shutdown()
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        pool.shutdown(wait=False, cancel_futures=True)
        for i in range(jobs):
            Lane(i).kill()
    results.sort(key=lambda r: (["uefi", "grub"].index(r["mode"]), r.get("order", 0)))
    total, failed = write_reports(results, args, started)
    print(f"TOTAL: {total - failed}/{total} passed; report: {Path(args.out_dir) / 'report.txt'}")
    return 0 if failed == 0 and total > 0 else 1


if __name__ == "__main__":
    sys.exit(main())
