from __future__ import annotations

import shlex
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from .log import LogBuffer
from .paths import CORE_SH, ROOT_DIR, SCRIPTS_DIR, TARGET_DIR

# Auto-install selftest: QEMU-only, never from the Windows GUI / win.ps1.
# Flashing that kernel is what formatted the host NVMe.
_BLOCKED_ON_WINDOWS = frozenset({"test-install"})


@dataclass(frozen=True)
class RunResult:
    ok: bool
    returncode: int


def windows_path_to_wsl(path: Path) -> str:
    resolved = path.resolve()
    drive = resolved.drive.rstrip(":").lower()
    return "/mnt/" + drive + resolved.as_posix()[2:]


def ensure_shell_scripts_lf(logs: LogBuffer | None = None) -> int:
    """WSL bash rejects `set -o pipefail` if scripts were checked out as CRLF."""
    converted = 0
    for path in SCRIPTS_DIR.rglob("*.sh"):
        data = path.read_bytes()
        if b"\r" not in data:
            continue
        path.write_bytes(data.replace(b"\r\n", b"\n").replace(b"\r", b""))
        converted += 1
    if converted and logs is not None:
        logs.add(
            "WARN",
            f"Converted {converted} shell script(s) from CRLF to LF (required by WSL bash).",
        )
    return converted


def wsl_core_cmd(action: str, *, extra_prefix: str = "") -> list[str]:
    """Invoke core.sh inside WSL with a hardware-safe environment.

    Always unsets POPCORN_CFLAGS so Windows cannot bake the QEMU auto-install
    kernel into popcorn-uefi.img. Guest disks are regular files under target/.
    """
    posix = windows_path_to_wsl(ROOT_DIR)
    remote = (
        f"cd {shlex.quote(posix)} || exit 1; "
        f"unset POPCORN_CFLAGS; "
        f'export PATH="$HOME/.cargo/bin:$PATH"; '
        f"{extra_prefix}"
        f"exec bash ./scripts/core.sh {shlex.quote(action)}"
    )
    return ["wsl", "-e", "bash", "-lc", remote]


def run_streamed(
    *,
    cmd: list[str],
    cwd: Path,
    env: dict[str, str] | None,
    logs: LogBuffer,
) -> RunResult:
    logs.add("CMD", " ".join(cmd))
    p = subprocess.Popen(
        cmd,
        cwd=str(cwd),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    assert p.stdout is not None
    for line in p.stdout:
        line = line.rstrip("\n")
        level = "ERROR" if "error:" in line.lower() or "undefined symbol" in line.lower() else "INFO"
        logs.add(level, line)
    rc = p.wait()
    return RunResult(ok=(rc == 0), returncode=rc)


def run_core(action: str, logs: LogBuffer, *, via_wsl: bool = False) -> RunResult:
    """Invoke scripts/core.sh <action>. On Windows, pass via_wsl=True."""
    TARGET_DIR.mkdir(parents=True, exist_ok=True)
    if via_wsl:
        if action in _BLOCKED_ON_WINDOWS:
            logs.add(
                "ERROR",
                f"{action} is blocked on Windows. It builds an auto-install kernel "
                "that must never be flashed. Run it only inside WSL QEMU if you need it.",
            )
            return RunResult(ok=False, returncode=2)
        ensure_shell_scripts_lf(logs)
        logs.add("INFO", "WSL: hardware-safe UEFI build (no POPCORN_TEST_INSTALL, QEMU uses target/ files only)")
        cmd = wsl_core_cmd(action)
        cwd = Path.cwd()
    else:
        cmd = ["bash", str(CORE_SH), action]
        cwd = ROOT_DIR
    return run_streamed(cmd=cmd, cwd=cwd, env=None, logs=logs)


def rm_rf(path: Path) -> None:
    if not path.exists():
        return
    if path.is_dir():
        for child in path.iterdir():
            rm_rf(child)
        path.rmdir()
        return
    path.unlink()


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def list_exists(paths: Iterable[Path]) -> list[Path]:
    return [p for p in paths if p.exists()]
