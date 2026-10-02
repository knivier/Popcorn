from __future__ import annotations

import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from .log import LogBuffer
from .paths import ROOT_DIR, TARGET_DIR
from .runner import run_core


def _windows_path_to_wsl(path: Path) -> str:
    resolved = path.resolve()
    drive = resolved.drive.rstrip(":").lower()
    return "/mnt/" + drive + resolved.as_posix()[2:]


@dataclass
class QemuConfig:
    memory_mb: int = 512
    cores: int = 2
    boot_from_cd: bool = True
    accel: str | None = None
    display: str | None = None


class QemuRunner:
    def __init__(self, *, logs: LogBuffer, via_wsl: bool = False):
        self.logs = logs
        self.via_wsl = via_wsl
        self.proc: subprocess.Popen[str] | None = None

    def running(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def stop(self) -> None:
        if self.proc and self.running():
            self.logs.add("WARN", "Stopping QEMU...")
            self.proc.terminate()
        self.proc = None
        if self.via_wsl:
            subprocess.run(
                ["wsl", "bash", "-lc", "killall qemu-system-x86_64 2>/dev/null || true"],
                check=False,
            )

    def run_uefi(self) -> None:
        if self.running():
            raise RuntimeError("QEMU already running")
        res = run_core("run-uefi", self.logs, via_wsl=self.via_wsl)
        if not res.ok:
            raise RuntimeError("run-uefi failed (see logs)")

    def run_iso(self, *, qemu_bin: str, iso_path: Path | None = None, cfg: QemuConfig) -> None:
        if self.running():
            raise RuntimeError("QEMU already running")

        if self.via_wsl:
            root_wsl = _windows_path_to_wsl(ROOT_DIR)
            cmd = ["wsl", "bash", "-lc", f"cd '{root_wsl}' && ./scripts/core.sh run-uefi"]
            self.logs.add("CMD", " ".join(cmd))
            self.proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            time.sleep(0.5)
            if self.proc.poll() is not None:
                out = self.proc.stdout.read() if self.proc.stdout else ""
                for line in (out or "").splitlines():
                    self.logs.add("ERROR", line)
                raise RuntimeError("QEMU/WSL exited immediately")
            return

        iso = iso_path or (TARGET_DIR / "popcorn.iso")
        if not iso.exists():
            raise RuntimeError("ISO not found — run build/iso first")

        def build_cmd(accel: str | None) -> list[str]:
            cmd = [qemu_bin]
            if accel:
                cmd += ["-accel", accel]
            cmd += [
                "-cdrom",
                str(iso),
                "-m",
                str(cfg.memory_mb),
                "-smp",
                str(cfg.cores),
                "-serial",
                "stdio",
            ]
            if cfg.display:
                cmd += ["-display", cfg.display]
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

        rc, out = start_or_raise(build_cmd(cfg.accel))
        if self.proc is not None:
            return

        out_l = (out or "").lower()
        if cfg.accel and ("invalid accelerator" in out_l or "invalid accel" in out_l):
            self.logs.add("WARN", f"Accel '{cfg.accel}' unsupported; retrying without acceleration.")
            rc2, _ = start_or_raise(build_cmd(None))
            if self.proc is not None:
                return
            raise RuntimeError(f"QEMU exited immediately (code {rc2}).")

        raise RuntimeError(f"QEMU exited immediately (code {rc}).")
