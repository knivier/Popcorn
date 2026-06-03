from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from .log import LogBuffer
from .runner import RunResult, ensure_dir, run_streamed
from .toolchain import Toolchain


@dataclass(frozen=True)
class BuildOutputs:
    kernel: Path
    obj_dir: Path


class KernelBuilder:
    def __init__(self, *, src_dir: Path, toolchain: Toolchain, logs: LogBuffer):
        self.src_dir = src_dir
        self.tc = toolchain
        self.logs = logs
        self.obj_dir = self.src_dir / "obj"
        self.kernel_out = self.src_dir / "kernel"

    def clean(self) -> None:
        for name in ("obj", "kernel", "isodir", "popcorn.iso"):
            p = self.src_dir / name
            if p.exists():
                if p.is_dir():
                    # Avoid importing shutil; use simple recursive removal here.
                    for child in sorted(p.rglob("*"), reverse=True):
                        if child.is_file() or child.is_symlink():
                            child.unlink(missing_ok=True)
                        else:
                            child.rmdir()
                    p.rmdir()
                else:
                    p.unlink(missing_ok=True)
        self.logs.add("OK", "Clean complete")

    def build(self) -> BuildOutputs:
        ensure_dir(self.obj_dir)

        asm = [
            ("core/kernel.asm", "kasm.o"),
            ("core/idt.asm", "idt.o"),
            ("core/context_switch.asm", "context_switch.o"),
        ]

        for src_rel, out_name in asm:
            res = run_streamed(
                cmd=[self.tc.nasm, "-f", "elf64", src_rel, "-o", str(self.obj_dir / out_name)],
                cwd=self.src_dir,
                env=None,
                logs=self.logs,
            )
            if not res.ok:
                raise RuntimeError(f"ASM build failed: {src_rel}")

        c_files = [
            ("core/kernel.c", "kc.o"),
            ("core/console.c", "console.o"),
            ("core/utils.c", "utils.o"),
            ("core/pop_module.c", "pop_module.o"),
            ("pops/shimjapii_pop.c", "shimjapii_pop.o"),
            ("pops/spinner_pop.c", "spinner_pop.o"),
            ("pops/uptime_pop.c", "uptime_pop.o"),
            ("pops/halt_pop.c", "halt_pop.o"),
            ("pops/filesystem_pop.c", "filesystem_pop.o"),
            ("core/multiboot2.c", "multiboot2.o"),
            ("pops/sysinfo_pop.c", "sysinfo_pop.o"),
            ("pops/memory_pop.c", "memory_pop.o"),
            ("pops/cpu_pop.c", "cpu_pop.o"),
            ("pops/dolphin_pop.c", "dolphin_pop.o"),
            ("core/timer.c", "timer.o"),
            ("core/scheduler.c", "scheduler.o"),
            ("core/memory.c", "memory.o"),
            ("core/vmm.c", "vmm.o"),
            ("core/init.c", "init.o"),
            ("core/syscall.c", "syscall.o"),
        ]

        cc_base = [self.tc.cc]
        # If we are on the clang fallback, target ELF.
        if self.tc.cc == "clang":
            cc_base += ["-target", "x86_64-unknown-elf"]

        cflags = [
            "-m64",
            "-c",
            "-Wall",
            "-Wextra",
            "-ffreestanding",
            "-fno-stack-protector",
            "-mcmodel=large",
            "-mno-red-zone",
        ]

        for src_rel, out_name in c_files:
            res = run_streamed(
                cmd=cc_base + cflags + [src_rel, "-o", str(self.obj_dir / out_name)],
                cwd=self.src_dir,
                env=None,
                logs=self.logs,
            )
            if not res.ok:
                raise RuntimeError(f"C build failed: {src_rel}")

        # Link
        link_ld = [self.tc.ld]
        if self.tc.ld == "ld.lld":
            link_ld += ["-m", "elf_x86_64"]
        else:
            link_ld += ["-m", "elf_x86_64"]

        objs = [str(self.obj_dir / name) for _, name in asm] + [str(self.obj_dir / name) for _, name in c_files]
        res = run_streamed(
            cmd=link_ld + ["-T", "link.ld", "-o", str(self.kernel_out)] + objs,
            cwd=self.src_dir,
            env=None,
            logs=self.logs,
        )
        if not res.ok:
            raise RuntimeError("Link failed")

        self.logs.add("OK", f"Kernel built: {self.kernel_out.name}")
        return BuildOutputs(kernel=self.kernel_out, obj_dir=self.obj_dir)

