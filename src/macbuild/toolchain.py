from __future__ import annotations

import shutil
from dataclasses import dataclass


@dataclass(frozen=True)
class Toolchain:
    cc: str
    ld: str
    nasm: str
    qemu: str
    mkrescue: str | None


def have(cmd: str) -> bool:
    return shutil.which(cmd) is not None


def detect_mkrescue() -> str | None:
    # Prefer BIOS-capable mkrescue for SeaBIOS/QEMU default.
    for candidate in (
        "grub2-mkrescue",
        "grub-mkrescue",
        "i686-elf-grub-mkrescue",
        "x86_64-elf-grub-mkrescue",
    ):
        if have(candidate):
            return candidate
    return None


def detect_toolchain() -> Toolchain | None:
    if not have("nasm") or not have("qemu-system-x86_64"):
        return None

    # Strongly prefer a real cross toolchain (this matches your “working previously” behavior).
    if have("x86_64-elf-gcc") and have("x86_64-elf-ld"):
        return Toolchain(
            cc="x86_64-elf-gcc",
            ld="x86_64-elf-ld",
            nasm="nasm",
            qemu="qemu-system-x86_64",
            mkrescue=detect_mkrescue(),
        )

    # Optional fallback (kept for portability), but macOS users should install the cross toolchain.
    if have("clang") and have("ld.lld"):
        return Toolchain(
            cc="clang",
            ld="ld.lld",
            nasm="nasm",
            qemu="qemu-system-x86_64",
            mkrescue=detect_mkrescue(),
        )

    return None

