from __future__ import annotations

import os
import shutil
import socket
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from .log import LogBuffer
from .paths import ROOT_DIR, TARGET_DIR
from .runner import run_core

VNC_HOST = "127.0.0.1"
VNC_PORT = 5900


def _windows_path_to_wsl(path: Path) -> str:
    resolved = path.resolve()
    drive = resolved.drive.rstrip(":").lower()
    return "/mnt/" + drive + resolved.as_posix()[2:]


def find_vnc_viewer() -> Path | None:
    """Locate TigerVNC / TightVNC / RealVNC viewer on Windows (or PATH)."""
    env = os.environ.get("POPCORN_VNC_VIEWER")
    if env and Path(env).is_file():
        return Path(env)

    which = shutil.which("vncviewer") or shutil.which("tvnviewer")
    if which:
        return Path(which)

    if sys.platform != "win32":
        return None

    candidates = [
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "TigerVNC" / "vncviewer.exe",
        Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "TigerVNC" / "vncviewer.exe",
        Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "TigerVNC" / "vncviewer.exe",
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "TightVNC" / "tvnviewer.exe",
        Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "TightVNC" / "tvnviewer.exe",
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "RealVNC" / "VNC Viewer" / "vncviewer.exe",
    ]
    for p in candidates:
        if p.is_file():
            return p
    return None


def wait_for_vnc(host: str = VNC_HOST, port: int = VNC_PORT, timeout_s: float = 45.0) -> bool:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.25)
    return False


def launch_vnc_viewer(logs: LogBuffer, host: str = VNC_HOST, port: int = VNC_PORT) -> bool:
    viewer = find_vnc_viewer()
    target = f"{host}:{port}"
    if viewer is None:
        logs.add("WARN", f"No VNC viewer found — connect manually to {target}")
        logs.add("INFO", "Install TigerVNC Viewer, or set POPCORN_VNC_VIEWER to vncviewer.exe")
        return False
    logs.add("INFO", f"Launching {viewer.name} → {target}")
    try:
        subprocess.Popen(
            [str(viewer), target],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return True
    except OSError as e:
        logs.add("ERROR", f"Failed to launch VNC viewer: {e}")
        return False


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
        else:
            subprocess.run(["killall", "qemu-system-x86_64"], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def run_uefi(self, *, open_vnc: bool = True) -> None:
        """Start UEFI QEMU. On Windows/WSL, force VNC and optionally open TigerVNC."""
        if self.running():
            raise RuntimeError("QEMU already running — click Stop QEMU first")

        # Clear stray guests from prior sessions (common after GUI relaunch).
        if self.via_wsl:
            subprocess.run(
                ["wsl", "bash", "-lc", "killall qemu-system-x86_64 2>/dev/null || true"],
                check=False,
            )

        if self.via_wsl:
            root_wsl = _windows_path_to_wsl(ROOT_DIR)
            cmd = [
                "wsl",
                "bash",
                "-lc",
                f"cd '{root_wsl}' && export POPCORN_QEMU_DISPLAY=vnc && ./scripts/core.sh run-uefi",
            ]
            self.logs.add("CMD", " ".join(cmd))
            self.logs.add("INFO", f"VNC will be at {VNC_HOST}:{VNC_PORT}")
            self.proc = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
            time.sleep(0.5)
            if self.proc.poll() is not None:
                out = self.proc.stdout.read() if self.proc.stdout else ""
                for line in (out or "").splitlines():
                    self.logs.add("ERROR", line)
                raise RuntimeError("QEMU/WSL exited immediately")

            if open_vnc:
                self.logs.add("INFO", f"Waiting for VNC on {VNC_HOST}:{VNC_PORT}…")
                if wait_for_vnc():
                    launch_vnc_viewer(self.logs)
                else:
                    self.logs.add("WARN", f"VNC not ready in time — connect manually to {VNC_HOST}:{VNC_PORT}")
            return

        # Native Linux/macOS: core.sh run-uefi (blocking interactive).
        res = run_core("run-uefi", self.logs, via_wsl=False)
        if not res.ok:
            raise RuntimeError("run-uefi failed (see logs)")

    def run_iso(self, *, qemu_bin: str, iso_path: Path | None = None, cfg: QemuConfig) -> None:
        if self.running():
            raise RuntimeError("QEMU already running — click Stop QEMU first")

        if self.via_wsl:
            # Legacy ISO button still boots the UEFI path under WSL with VNC + viewer.
            self.run_uefi(open_vnc=True)
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
