#!/usr/bin/env python3
"""Popcorn shell test harness (run via ./scripts/core.sh test-shell).

Boots a kernel built with -DPOPCORN_SERIAL_CONSOLE under QEMU with COM1 on a
local TCP socket. That build mirrors scrolling-console text to COM1 and treats
COM1 bytes as typed keys, so every shell command can be driven and checked
from here without screenshots.

Usage (inside WSL / Linux, from the repo root, after the test images exist):
  python3 scripts/shell_test.py [--boot uefi|grub|both] [--only NAME ...]
                                [--explore "cmd1" "cmd2" ...] [--list]
"""

from __future__ import annotations

import argparse
import os
import re
import socket
import subprocess
import sys
import time
import traceback
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CORE = ROOT / "scripts" / "core.sh"
TARGET = ROOT / "target"
PROMPT_RE = re.compile(r"popcorn@kernel:[^\n$]*\$ $")
PANIC_RE = re.compile(r"#PF|#GP|#DF|#UD|EXCEPTION|PANIC|panicked|KERNEL PANIC", re.I)
ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


class GuestError(Exception):
    pass


class Guest:
    """One QEMU boot with COM1 on TCP."""

    def __init__(self, boot: str, port: int, log_path: Path):
        self.boot = boot
        self.port = port
        self.buf = ""
        self.log = open(log_path, "a", encoding="utf-8", errors="replace")
        self.sock: socket.socket | None = None

    # -- lifecycle ---------------------------------------------------------
    def start(self, boot_timeout: float = 180.0) -> None:
        sub = "_shell-boot" if self.boot == "uefi" else "_shell-boot-legacy"
        self.log.write(f"\n===== boot {self.boot} {datetime.now().isoformat()} =====\n")
        subprocess.run([str(CORE), sub, str(self.port)], check=True, cwd=ROOT,
                       stdout=self.log, stderr=subprocess.STDOUT)
        deadline = time.time() + 15
        while True:
            try:
                self.sock = socket.create_connection(("127.0.0.1", self.port), timeout=2)
                break
            except OSError:
                if time.time() > deadline:
                    raise GuestError("could not connect to QEMU serial socket")
                time.sleep(0.2)
        self.sock.settimeout(0.2)
        # Boot is done once the first prompt is drawn on the scrolling console.
        self.read_until(PROMPT_RE, boot_timeout, what="first prompt")
        time.sleep(0.3)
        self.drain()

    def stop(self) -> None:
        if self.sock:
            try:
                self.sock.close()
            except OSError:
                pass
            self.sock = None
        subprocess.run([str(CORE), "_shell-kill"], cwd=ROOT,
                       stdout=self.log, stderr=subprocess.STDOUT)

    def alive(self) -> bool:
        try:
            pid = int((Path(os.environ.get("RUNTIME_DIR", "/tmp/popcorn-qemu"))
                       / "popcorn-qemu.pid").read_text().strip())
            os.kill(pid, 0)
            return True
        except (OSError, ValueError):
            return False

    # -- I/O ---------------------------------------------------------------
    def _recv(self) -> str:
        assert self.sock
        try:
            data = self.sock.recv(65536)
        except socket.timeout:
            return ""
        if not data:
            raise GuestError("serial socket closed (guest died?)")
        text = ANSI_RE.sub("", data.decode("latin-1")).replace("\r", "")
        self.log.write(text)
        self.log.flush()
        return text

    def drain(self) -> str:
        out = ""
        while True:
            t = self._recv()
            if not t:
                break
            out += t
        self.buf += out
        return out

    def read_until(self, pattern: re.Pattern, timeout: float, what: str = "") -> str:
        deadline = time.time() + timeout
        while True:
            m = pattern.search(self.buf)
            if m:
                got = self.buf[: m.end()]
                self.buf = self.buf[m.end():]
                return got
            if time.time() > deadline:
                tail = self.buf[-600:]
                raise GuestError(f"timeout waiting for {what or pattern.pattern!r}; tail:\n{tail}")
            self.buf += self._recv()

    def send_raw(self, text: str, pace: float = 0.004) -> None:
        assert self.sock
        for ch in text:
            self.sock.sendall(ch.encode("latin-1"))
            time.sleep(pace)

    def sendkey(self, *keys: str, pace: float = 0.08) -> None:
        """Press keys through the emulated PS/2 keyboard (QEMU HMP sendkey)."""
        mon = Path(os.environ.get("RUNTIME_DIR", "/tmp/popcorn-qemu")) / "shell-mon.sock"
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(3)
        s.connect(str(mon))
        try:
            for k in keys:
                s.sendall(f"sendkey {k}\n".encode())
                time.sleep(pace)
            time.sleep(0.2)
        finally:
            s.close()

    def type_ps2(self, text: str) -> None:
        """Type ASCII through the PS/2 keyboard (letters, digits, space, basic punctuation)."""
        names = {" ": "spc", ".": "dot", "-": "minus", "/": "slash", ",": "comma",
                 ":": "shift-semicolon", "!": "shift-1", "=": "equal", "_": "shift-minus"}
        keys = []
        for ch in text:
            if ch in names:
                keys.append(names[ch])
            elif ch.isupper():
                keys.append("shift-" + ch.lower())
            else:
                keys.append(ch)
        self.sendkey(*keys)

    def wait_prompt(self, timeout: float = 30.0) -> str:
        return self.read_until(PROMPT_RE, timeout, what="prompt")

    def cmd(self, line: str, timeout: float = 30.0) -> str:
        """Type a command, press Enter, return its output (between echo and prompt)."""
        self.drain()
        self.buf = ""
        self.send_raw(line)
        self.send_raw("\r")
        raw = self.read_until(PROMPT_RE, timeout, what=f"prompt after {line!r}")
        # Strip the echoed command line and the trailing prompt.
        nl = raw.find("\n")
        body = raw[nl + 1:] if nl >= 0 else raw
        body = PROMPT_RE.sub("", body)
        if PANIC_RE.search(body):
            raise GuestError(f"kernel fault text after {line!r}:\n{body[-800:]}")
        return body.strip("\n")


# ---------------------------------------------------------------------------
# Test registry
# ---------------------------------------------------------------------------
TESTS: list[tuple[str, set[str], object]] = []


def test(name: str, boots: set[str] | None = None):
    def deco(fn):
        TESTS.append((name, boots or {"uefi", "grub"}, fn))
        return fn
    return deco


def expect(out: str, *patterns: str, cmd: str = "") -> None:
    for p in patterns:
        if not re.search(p, out, re.I | re.M):
            raise AssertionError(f"{cmd!r}: expected /{p}/ in output:\n{out[-1500:]}")


def forbid(out: str, *patterns: str, cmd: str = "") -> None:
    for p in patterns:
        if re.search(p, out, re.I | re.M):
            raise AssertionError(f"{cmd!r}: unexpected /{p}/ in output:\n{out[-1500:]}")


def run(g: Guest, line: str, *patterns: str, no: tuple[str, ...] = (), timeout: float = 30.0) -> str:
    out = g.cmd(line, timeout=timeout)
    expect(out, *patterns, cmd=line)
    forbid(out, *no, cmd=line)
    return out


from shell_tests import register  # noqa: E402  (test definitions live next door)

register(test, run, expect, forbid)


# ---------------------------------------------------------------------------
def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--boot", choices=["uefi", "grub", "both"], default="both")
    ap.add_argument("--only", nargs="*", default=[])
    ap.add_argument("--explore", nargs="*")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--port", type=int, default=45454)
    args = ap.parse_args()

    if args.list:
        for name, boots, _ in TESTS:
            print(f"{name:32s} {','.join(sorted(boots))}")
        return 0

    log_path = TARGET / "shell-test.log"
    log_path.write_text("")
    boots = ["uefi", "grub"] if args.boot == "both" else [args.boot]

    if args.explore is not None:
        g = Guest(boots[0], args.port, log_path)
        try:
            g.start()
            for c in args.explore:
                print(f"\n>>> {c}")
                try:
                    print(g.cmd(c, timeout=60))
                except GuestError as e:
                    print(f"!!! {e}")
        finally:
            g.stop()
        return 0

    failures: list[str] = []
    passed = 0
    for boot in boots:
        selected = [t for t in TESTS if boot in t[1] and (not args.only or any(o in t[0] for o in args.only))]
        if not selected:
            continue
        print(f"== {boot}: {len(selected)} tests ==", flush=True)
        g = Guest(boot, args.port, log_path)
        try:
            g.start()
        except Exception as e:  # noqa: BLE001
            failures.append(f"{boot}: boot failed: {e}")
            print(f"FAIL {boot} boot: {e}", flush=True)
            g.stop()
            continue
        for name, _, fn in selected:
            t0 = time.time()
            try:
                if g.sock is None:
                    g.start()
                fn(g)
                passed += 1
                print(f"PASS {boot}:{name} ({time.time() - t0:.1f}s)", flush=True)
            except Exception as e:  # noqa: BLE001
                msg = f"{boot}:{name}: {e}"
                failures.append(msg)
                print(f"FAIL {msg}", flush=True)
                if os.environ.get("SHELL_TEST_TRACE"):
                    traceback.print_exc()
                # Reboot for the next test if the guest died or wedged.
                g.stop()
                g.sock = None
        g.stop()

    print(f"\n{passed} passed, {len(failures)} failed (log: {log_path})")
    for f in failures:
        print(f"  - {f.splitlines()[0]}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    sys.exit(main())
