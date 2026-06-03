from __future__ import annotations

from pathlib import Path

from .log import LogBuffer
from .runner import ensure_dir, run_streamed, write_text
from .toolchain import Toolchain


class IsoBuilder:
    def __init__(self, *, src_dir: Path, toolchain: Toolchain, logs: LogBuffer):
        self.src_dir = src_dir
        self.tc = toolchain
        self.logs = logs
        self.iso_out = self.src_dir / "popcorn.iso"

    def create(self, kernel_path: Path) -> Path:
        if not kernel_path.exists():
            raise RuntimeError("kernel not found; build first")

        mkrescue = self.tc.mkrescue
        if not mkrescue:
            raise RuntimeError("mkrescue not found (need i686-elf-grub-mkrescue on macOS)")

        isodir = self.src_dir / "isodir"
        if isodir.exists():
            # cleanup best-effort
            for child in sorted(isodir.rglob("*"), reverse=True):
                if child.is_file() or child.is_symlink():
                    child.unlink(missing_ok=True)
                else:
                    child.rmdir()
            isodir.rmdir()

        ensure_dir(isodir / "boot" / "grub")
        (isodir / "boot" / "kernel").write_bytes(kernel_path.read_bytes())

        write_text(
            isodir / "boot" / "grub" / "grub.cfg",
            "\n".join(
                [
                    "insmod all_video",
                    "insmod efi_gop",
                    "set gfxmode=1024x768x32",
                    "set gfxpayload=1024x768x32",
                    "",
                    "set timeout=3",
                    "set default=0",
                    "",
                    'menuentry "Popcorn Kernel x64" {',
                    '    echo "Loading Popcorn kernel..."',
                    "    multiboot2 /boot/kernel",
                    '    echo "Booting kernel..."',
                    "    boot",
                    "}",
                    "",
                ]
            ),
        )

        res = run_streamed(
            cmd=[mkrescue, "-o", str(self.iso_out), str(isodir)],
            cwd=self.src_dir,
            env=None,
            logs=self.logs,
        )
        if not res.ok or not self.iso_out.exists():
            raise RuntimeError("ISO creation failed")

        # cleanup isodir
        for child in sorted(isodir.rglob("*"), reverse=True):
            if child.is_file() or child.is_symlink():
                child.unlink(missing_ok=True)
            else:
                child.rmdir()
        isodir.rmdir()

        self.logs.add("OK", f"ISO created: {self.iso_out.name}")
        return self.iso_out

