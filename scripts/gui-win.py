#!/usr/bin/env python3
"""
Popcorn Windows GUI — builds/runs via WSL2.

Always builds the hardware-safe UEFI image (no auto-install kernel).
QEMU attaches only regular files under target/ — never PhysicalDrive / the
Windows NVMe. test-install is blocked from this GUI.

Run: python scripts/gui-win.py
CLI: powershell -File scripts/win.ps1 all
Requires: WSL2 distro with nasm clang lld qemu-system-x86 ovmf/edk2-ovmf mtools rustup.
"""
from __future__ import annotations

import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from popcorn_build.gui_tk import PopcornTkGui, install_hint_for


def main() -> int:
    if not shutil.which("wsl"):
        print("WSL not found. Install WSL2, then open this GUI again.")
        return 2
    PopcornTkGui(
        title="Popcorn Builder — Windows (WSL)",
        install_hint=install_hint_for("windows"),
        via_wsl=True,
    ).run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
