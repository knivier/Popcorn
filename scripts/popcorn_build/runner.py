from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from .log import LogBuffer
from .paths import CORE_SH, ROOT_DIR, TARGET_DIR


@dataclass(frozen=True)
class RunResult:
    ok: bool
    returncode: int


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
        # Convert Windows path to /mnt/<drive>/... for WSL.
        root = ROOT_DIR.resolve()
        drive = root.drive.rstrip(":").lower()
        posix = "/mnt/" + drive + root.as_posix()[2:]
        cmd = ["wsl", "bash", f"{posix}/scripts/core.sh", action]
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
