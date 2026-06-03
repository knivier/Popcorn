from __future__ import annotations

import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from .log import LogBuffer


@dataclass
class QemuConfig:
    memory_mb: int = 512
    cores: int = 2
    # “Significantly bigger”: for text-mode kernels the *content* is still 80x25,
    # but we can make QEMU’s window large and keep it from shrinking by using SDL.
    # On macOS, SDL display backend isn't always available; cocoa is. We’ll keep cocoa
    # but increase RAM/cores and force CD boot. If you later add a graphics mode, the
    # window will use that resolution.
    boot_from_cd: bool = True
    # Some Homebrew QEMU builds don't support HVF; default to no accel (TCG).
    accel: str | None = None


class QemuRunner:
    def __init__(self, *, logs: LogBuffer):
        self.logs = logs
        self.proc: subprocess.Popen[str] | None = None

    def running(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def stop(self) -> None:
        if self.proc and self.running():
            self.logs.add("WARN", "Stopping QEMU...")
            self.proc.terminate()
        self.proc = None

    def run_iso(self, *, qemu_bin: str, iso_path: Path, cfg: QemuConfig) -> None:
        if self.running():
            raise RuntimeError("QEMU already running")
        if not iso_path.exists():
            raise RuntimeError("ISO not found")

        def build_cmd(accel: str | None) -> list[str]:
            cmd = [qemu_bin]
            if accel:
                cmd += ["-accel", accel]
            cmd += [
                "-cdrom",
                str(iso_path),
                "-m",
                str(cfg.memory_mb),
                "-smp",
                str(cfg.cores),
                "-serial",
                "stdio",
                "-display",
                "cocoa",
            ]
            if cfg.boot_from_cd:
                cmd += ["-boot", "d"]
            return cmd

        def start_or_raise(cmd: list[str]) -> tuple[int, str]:
            self.logs.add("CMD", " ".join(cmd))
            proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            time.sleep(0.2)
            if proc.poll() is None:
                self.proc = proc
                return (0, "")

            out = ""
            try:
                if proc.stdout:
                    out = proc.stdout.read() or ""
            except Exception:
                out = ""
            rc = proc.returncode
            if out.strip():
                for line in out.splitlines():
                    self.logs.add("ERROR", line)
            return (rc, out)

        # Try preferred accel first; if it fails with “invalid accelerator”, retry without accel.
        rc, out = start_or_raise(build_cmd(cfg.accel))
        if self.proc is not None:
            return

        out_l = (out or "").lower()
        if cfg.accel and ("invalid accelerator" in out_l or "invalid accel" in out_l or "invalid accelerator hvf" in out_l):
            self.logs.add("WARN", f"Accel '{cfg.accel}' unsupported; retrying without acceleration.")
            rc2, out2 = start_or_raise(build_cmd(None))
            if self.proc is not None:
                return
            # Surface second failure output too.
            if out2.strip():
                for line in out2.splitlines():
                    self.logs.add("ERROR", line)
            raise RuntimeError(f"QEMU exited immediately (code {rc2}).")

        raise RuntimeError(f"QEMU exited immediately (code {rc}).")

