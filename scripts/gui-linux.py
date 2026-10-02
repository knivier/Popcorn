#!/usr/bin/env python3
"""Popcorn Linux Tkinter GUI. Run: python3 scripts/gui-linux.py"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from popcorn_build.gui_tk import PopcornTkGui, install_hint_for


def main() -> int:
    PopcornTkGui(
        title="Popcorn Builder — Linux",
        install_hint=install_hint_for("linux"),
        via_wsl=False,
    ).run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
