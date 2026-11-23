from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from .log import LogBuffer
from .paths import TARGET_DIR
from .runner import ensure_dir, run_core, rm_rf


@dataclass(frozen=True)
class BuildOutputs:
    kernel: Path
    obj_dir: Path


class KernelBuilder:
    """Thin wrapper around scripts/core.sh (build / all / clean).

    There is no direct-toolchain fallback: a failed core.sh build raises.
    """

    def __init__(self, *, logs: LogBuffer, via_wsl: bool = False):
        self.logs = logs
        self.via_wsl = via_wsl
        self.obj_dir = TARGET_DIR / "obj"
        self.kernel_out = TARGET_DIR / "kernel"

    def clean(self) -> None:
        res = run_core("clean", self.logs, via_wsl=self.via_wsl)
        if not res.ok:
            rm_rf(TARGET_DIR)
            ensure_dir(TARGET_DIR)
        self.logs.add("OK", "Clean complete")

    def build(self) -> BuildOutputs:
        res = run_core("build", self.logs, via_wsl=self.via_wsl)
        if not res.ok:
            raise RuntimeError("Kernel build failed (see logs)")
        if not self.kernel_out.exists():
            raise RuntimeError(f"Kernel missing after build: {self.kernel_out}")
        self.logs.add("OK", f"Kernel built: {self.kernel_out}")
        return BuildOutputs(kernel=self.kernel_out, obj_dir=self.obj_dir)

    def build_all(self) -> BuildOutputs:
        res = run_core("all", self.logs, via_wsl=self.via_wsl)
        if not res.ok:
            raise RuntimeError("UEFI all-build failed (see logs)")
        return BuildOutputs(kernel=self.kernel_out, obj_dir=self.obj_dir)
