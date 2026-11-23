from __future__ import annotations

from pathlib import Path

from .log import LogBuffer
from .paths import TARGET_DIR
from .runner import run_core


class IsoBuilder:
    """Thin wrapper around `scripts/core.sh iso` (legacy GRUB ISO in target/)."""

    def __init__(self, *, logs: LogBuffer, via_wsl: bool = False):
        self.logs = logs
        self.via_wsl = via_wsl
        self.iso_out = TARGET_DIR / "popcorn.iso"

    def create(self) -> Path:
        res = run_core("iso", self.logs, via_wsl=self.via_wsl)
        if not res.ok or not self.iso_out.exists():
            raise RuntimeError("ISO creation failed")
        self.logs.add("OK", f"ISO created: {self.iso_out}")
        return self.iso_out
