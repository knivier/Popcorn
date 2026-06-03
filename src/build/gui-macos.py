#!/usr/bin/env python3
"""
Popcorn macOS GUI builder (WebView edition).

Run from anywhere:
  python3 src/build/gui-macos.py
"""

from __future__ import annotations

import sys
from pathlib import Path
from threading import Lock, Thread

BUILD_DIR = Path(__file__).resolve().parent
SRC_DIR = BUILD_DIR.parent
sys.path.insert(0, str(BUILD_DIR))


def _read_ui() -> str:
    ui_dir = BUILD_DIR / "popcorn_build" / "ui"
    html = (ui_dir / "index.html").read_text()
    css = (ui_dir / "styles.css").read_text()
    js = (ui_dir / "app.js").read_text()
    html = html.replace('<link rel="stylesheet" href="styles.css" />', f"<style>{css}</style>")
    html = html.replace("<script src=\"app.js\"></script>", f"<script>{js}</script>")
    return html


def main() -> int:
    if not (SRC_DIR / "core" / "kernel.asm").exists():
        print("Could not find kernel sources (expected src/core/kernel.asm).")
        return 2

    try:
        import webview  # pywebview
    except Exception:
        print("Missing dependency: pywebview")
        print("Install: pip3 install pywebview")
        return 3

    from popcorn_build.builder import KernelBuilder
    from popcorn_build.iso import IsoBuilder
    from popcorn_build.log import LogBuffer
    from popcorn_build.qemu import QemuConfig, QemuRunner
    from popcorn_build.toolchain import Toolchain, detect_mkrescue, detect_toolchain, have

    logs = LogBuffer()

    state_lock = Lock()
    busy = False
    status = "Ready"
    last_level = "OK"

    tc = detect_toolchain()
    if not tc:
        logs.add("ERROR", "Missing toolchain/tools.")
        logs.add(
            "INFO",
            "Install (macOS): brew install nasm qemu xorriso mtools i686-elf-grub x86_64-elf-grub x86_64-elf-binutils x86_64-elf-gcc",
        )
        tc = Toolchain(
            cc="x86_64-elf-gcc",
            ld="x86_64-elf-ld",
            nasm="nasm",
            qemu="qemu-system-x86_64",
            mkrescue=detect_mkrescue(),
        )

    # macOS BIOS boot (SeaBIOS): prefer i686-elf-grub-mkrescue when available.
    if tc.mkrescue == "x86_64-elf-grub-mkrescue" and have("i686-elf-grub-mkrescue"):
        tc = Toolchain(cc=tc.cc, ld=tc.ld, nasm=tc.nasm, qemu=tc.qemu, mkrescue="i686-elf-grub-mkrescue")

    builder = KernelBuilder(src_dir=SRC_DIR, toolchain=tc, logs=logs)
    iso = IsoBuilder(src_dir=SRC_DIR, toolchain=tc, logs=logs)
    qemu = QemuRunner(logs=logs)

    def set_state(level: str, msg: str) -> None:
        nonlocal status, last_level
        with state_lock:
            status = msg
            last_level = level

    def set_busy(v: bool) -> None:
        nonlocal busy
        with state_lock:
            busy = v

    def run_bg(fn):
        def _runner():
            try:
                set_busy(True)
                fn()
            except Exception as e:
                logs.add("ERROR", str(e))
                set_state("ERROR", "Failed")
            finally:
                set_busy(False)

        Thread(target=_runner, daemon=True).start()

    class Api:
        def get_state(self):
            with state_lock:
                return {
                    "busy": busy,
                    "status": status,
                    "last_level": last_level,
                    "qemu_running": qemu.running(),
                    "toolchain": f"{tc.cc} + {tc.ld}",
                    "mkrescue": tc.mkrescue or "",
                }

        def poll_logs(self, since_idx: int):
            return [e.__dict__ for e in logs.snapshot(int(since_idx))]

        def clear_logs(self):
            logs.clear()
            return True

        def stop_qemu(self):
            qemu.stop()
            return True

        def run_action(self, action: str):
            action = str(action)
            if action not in ("build", "iso", "recomp", "run"):
                return False

            def _do():
                set_state("INFO", "Running…")
                if action == "build":
                    logs.add("INFO", "Build started")
                    builder.build()
                    set_state("OK", "Build complete")
                    return
                if action == "iso":
                    logs.add("INFO", "ISO started")
                    out = builder.build()
                    iso.create(out.kernel)
                    set_state("OK", "ISO complete")
                    return
                if action == "run":
                    logs.add("INFO", "QEMU run started")
                    iso_path = SRC_DIR / "popcorn.iso"
                    if not iso_path.exists():
                        raise RuntimeError("popcorn.iso not found (create ISO first)")
                    qemu.run_iso(
                        qemu_bin=tc.qemu,
                        iso_path=iso_path,
                        cfg=QemuConfig(memory_mb=512, cores=2, boot_from_cd=True),
                    )
                    if qemu.running():
                        set_state("OK", "QEMU running")
                    else:
                        raise RuntimeError("QEMU did not start (see logs).")
                    return

                logs.add("INFO", "Recomp started")
                builder.clean()
                out = builder.build()
                iso_path = iso.create(out.kernel)

                # QEMU “bigger” request: for text-mode kernels, the content is still 80x25,
                # but we can at least allocate more space and force CD boot reliably.
                qemu.run_iso(
                    qemu_bin=tc.qemu,
                    iso_path=iso_path,
                    cfg=QemuConfig(memory_mb=512, cores=2, boot_from_cd=True),
                )
                if qemu.running():
                    set_state("OK", "QEMU running")
                else:
                    set_state("OK", "Recomp complete")

            run_bg(_do)
            return True

    window = webview.create_window(
        "Popcorn Builder",
        html=_read_ui(),
        width=1180,
        height=760,
        frameless=False,
        easy_drag=False,
        js_api=Api(),
    )

    def pump_qemu_logs():
        while True:
            proc = qemu.proc
            if proc and proc.stdout:
                line = proc.stdout.readline()
                if not line:
                    if proc.poll() is not None:
                        logs.add("INFO", "QEMU session ended")
                        if qemu.proc is proc:
                            qemu.proc = None
                    continue
                logs.add("INFO", line.rstrip("\n"))

    Thread(target=pump_qemu_logs, daemon=True).start()
    webview.start(debug=False)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

