from __future__ import annotations

from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent.parent
ROOT_DIR = SCRIPTS_DIR.parent
SRC_DIR = ROOT_DIR / "src"
TARGET_DIR = ROOT_DIR / "target"
CORE_SH = SCRIPTS_DIR / "core.sh"

__all__ = [
    "SCRIPTS_DIR",
    "ROOT_DIR",
    "SRC_DIR",
    "TARGET_DIR",
    "CORE_SH",
    "builder",
    "iso",
    "log",
    "paths",
    "qemu",
    "runner",
    "toolchain",
]
