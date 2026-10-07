"""Shared Tkinter GUI for Fedora / Linux / Windows."""
from __future__ import annotations

import platform
import subprocess
import threading
import tkinter as tk
from tkinter import messagebox, scrolledtext
from typing import Callable

from .builder import KernelBuilder
from .iso import IsoBuilder
from .log import LogBuffer
from .paths import ROOT_DIR, SRC_DIR, TARGET_DIR
from .qemu import QemuConfig, QemuRunner
from .runner import run_core
from .toolchain import Toolchain, detect_toolchain


class PopcornTkGui:
    def __init__(self, *, title: str, install_hint: str, via_wsl: bool = False):
        if not (SRC_DIR / "core" / "kernel.asm").exists():
            raise SystemExit(f"Could not find kernel sources at {SRC_DIR}")

        self.via_wsl = via_wsl
        self.install_hint = install_hint
        self.logs = LogBuffer()
        self.tc = detect_toolchain() or Toolchain(
            cc="clang", ld="ld.lld", nasm="nasm", qemu="qemu-system-x86_64", mkrescue=None
        )
        self.builder = KernelBuilder(toolchain=self.tc, logs=self.logs, via_wsl=via_wsl)
        self.iso = IsoBuilder(toolchain=self.tc, logs=self.logs, via_wsl=via_wsl)
        self.qemu = QemuRunner(logs=self.logs, via_wsl=via_wsl)

        self.root = tk.Tk()
        self.root.title(title)
        self.root.geometry("980x720")
        self.colors = {
            "bg": "#1a1a1a",
            "surface": "#2d2d2d",
            "primary": "#00d4aa",
            "text": "#ffffff",
            "muted": "#b2bec3",
            "error": "#e17055",
            "ok": "#00b894",
        }
        self.root.configure(bg=self.colors["bg"])
        self._busy = False
        self._build_ui()
        self._poll_logs()

    def _build_ui(self) -> None:
        header = tk.Frame(self.root, bg=self.colors["surface"], height=72)
        header.pack(fill=tk.X, padx=16, pady=16)
        header.pack_propagate(False)
        tk.Label(
            header,
            text="Popcorn Builder",
            font=("Segoe UI", 18, "bold"),
            bg=self.colors["surface"],
            fg=self.colors["text"],
        ).pack(side=tk.LEFT, padx=16, pady=16)
        self.status = tk.Label(
            header,
            text="Ready",
            font=("Segoe UI", 11),
            bg=self.colors["surface"],
            fg=self.colors["primary"],
        )
        self.status.pack(side=tk.RIGHT, padx=16)

        body = tk.Frame(self.root, bg=self.colors["bg"])
        body.pack(fill=tk.BOTH, expand=True, padx=16, pady=(0, 16))

        left = tk.Frame(body, bg=self.colors["bg"], width=260)
        left.pack(side=tk.LEFT, fill=tk.Y, padx=(0, 16))
        right = tk.Frame(body, bg=self.colors["bg"])
        right.pack(side=tk.RIGHT, fill=tk.BOTH, expand=True)

        def btn(text: str, cmd: Callable[[], None], primary: bool = False) -> None:
            b = tk.Button(
                left,
                text=text,
                command=cmd,
                bg=self.colors["primary"] if primary else self.colors["surface"],
                fg=self.colors["text"],
                font=("Segoe UI", 11, "bold"),
                relief=tk.FLAT,
                padx=12,
                pady=10,
                cursor="hand2",
            )
            b.pack(fill=tk.X, pady=6)

        btn("Full Automation\n(clean → all → run-uefi)", self._full, primary=True)
        btn("Build kernel", lambda: self._run("build"))
        btn("Build all (UEFI img)", lambda: self._run("all"))
        btn("Legacy ISO", lambda: self._run("iso"))
        btn("Run UEFI (QEMU + VNC)", lambda: self._run("run-uefi"))
        btn("Stop QEMU", self._stop_qemu)
        btn("Clean target/", lambda: self._run("clean"))
        btn("Clear logs", self._clear_logs)

        tk.Label(
            left,
            text=f"Output: {TARGET_DIR}",
            wraplength=240,
            justify=tk.LEFT,
            bg=self.colors["bg"],
            fg=self.colors["muted"],
            font=("Segoe UI", 8),
        ).pack(anchor="w", pady=(20, 0))

        self.log_box = scrolledtext.ScrolledText(
            right,
            bg="#111",
            fg=self.colors["text"],
            insertbackground=self.colors["text"],
            font=("Consolas", 10),
            relief=tk.FLAT,
        )
        self.log_box.pack(fill=tk.BOTH, expand=True)
        self._log_idx = -1
        self.logs.add("INFO", f"Root: {ROOT_DIR}")
        self.logs.add("INFO", f"Toolchain: {self.tc.cc} + {self.tc.ld}" + (" (via WSL)" if self.via_wsl else ""))
        self.logs.add("INFO", self.install_hint)
        if self.via_wsl:
            self.logs.add(
                "INFO",
                "Windows: hardware-safe img only (no auto-install). QEMU uses target/ files — never the Windows disk.",
            )

    def _set_status(self, text: str, ok: bool = True) -> None:
        self.status.config(text=text, fg=self.colors["ok"] if ok else self.colors["error"])

    def _poll_logs(self) -> None:
        for ev in self.logs.snapshot(self._log_idx):
            self._log_idx = ev.idx
            self.log_box.insert(tk.END, f"[{ev.level}] {ev.message}\n")
            self.log_box.see(tk.END)
        self.root.after(200, self._poll_logs)

    def _clear_logs(self) -> None:
        self.logs.clear()
        self.log_box.delete("1.0", tk.END)
        self._log_idx = -1

    def _stop_qemu(self) -> None:
        self.qemu.stop()
        self.logs.add("WARN", "QEMU stop requested")
        self._set_status("Stopped")

    def _bg(self, fn: Callable[[], None]) -> None:
        if self._busy:
            messagebox.showinfo("Busy", "A build/run is already in progress.")
            return

        def runner() -> None:
            self._busy = True
            try:
                fn()
            except Exception as e:
                self.logs.add("ERROR", str(e))
                self.root.after(0, lambda: self._set_status("Failed", ok=False))
            finally:
                self._busy = False

        threading.Thread(target=runner, daemon=True).start()

    def _run(self, action: str) -> None:
        def do() -> None:
            self.root.after(0, lambda: self._set_status(f"Running {action}…"))
            if action == "run-uefi":
                res = run_core("all", self.logs, via_wsl=self.via_wsl)
                if not res.ok:
                    raise RuntimeError("Build failed before run")
                # WSL: VNC + auto-launch TigerVNC when ready.
                self.qemu.run_uefi(open_vnc=True)
                self.root.after(0, lambda: self._set_status("QEMU + VNC running"))
                return
            res = run_core(action, self.logs, via_wsl=self.via_wsl)
            if not res.ok:
                raise RuntimeError(f"{action} failed")
            self.root.after(0, lambda: self._set_status(f"{action} ok"))

        self._bg(do)

    def _full(self) -> None:
        def do() -> None:
            self.root.after(0, lambda: self._set_status("Full automation…"))
            for step in ("clean", "all"):
                res = run_core(step, self.logs, via_wsl=self.via_wsl)
                if not res.ok:
                    raise RuntimeError(f"{step} failed")
            self.qemu.run_uefi(open_vnc=True)
            self.root.after(0, lambda: self._set_status("QEMU + VNC running"))

        self._bg(do)

    def run(self) -> None:
        self.root.mainloop()


def install_hint_for(platform_name: str) -> str:
    name = platform_name.lower()
    if name in ("fedora", "rhel"):
        return "Install: sudo dnf install nasm clang lld qemu-system-x86 grub2-tools-extra xorriso mtools edk2-ovmf dosfstools"
    if name in ("linux", "debian", "ubuntu"):
        return "Install: sudo apt install nasm clang lld qemu-system-x86 grub-pc-bin xorriso mtools ovmf dosfstools"
    if name in ("win", "windows"):
        return (
            "Uses WSL2. In Fedora/Ubuntu WSL: install nasm clang lld qemu-system-x86 "
            "edk2-ovmf (or ovmf) mtools rustup. Builds popcorn-uefi.img only (never auto-install). "
            "QEMU never attaches the Windows drive."
        )
    return "See readme.md for platform toolchain packages."


def detect_via_wsl() -> bool:
    return platform.system().lower() == "windows"
