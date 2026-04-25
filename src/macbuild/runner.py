from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from .log import LogBuffer


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

