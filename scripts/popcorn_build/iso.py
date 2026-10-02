from __future__ import annotations

from pathlib import Path

from .log import LogBuffer
from .paths import TARGET_DIR
from .runner import run_core
from .toolchain import Toolchain


class IsoBuilder:
    def __init__(self, *, src_dir: Path | None = None, toolchain: Toolchain, logs: LogBuffer, via_wsl: bool = False):
        self.tc = toolchain
        self.logs = logs
        self.via_wsl = via_wsl
        self.iso_out = TARGET_DIR / "popcorn.iso"

    def create(self, kernel_path: Path | None = None) -> Path:
        res = run_core("iso", self.logs, via_wsl=self.via_wsl)
        if not res.ok or not self.iso_out.exists():
            raise RuntimeError("ISO creation failed")
        self.logs.add("OK", f"ISO created: {self.iso_out}")
        return self.iso_out
