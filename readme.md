# Welcome to Popcorn

A modern, modular 64-bit kernel framework designed for learning operating system development.

**Current Version: v0.7**

## Quick Start

All tooling lives under `scripts/`. All build outputs go to `target/`.

### CLI (recommended)

```bash
./scripts/core.sh all        # kernel + UEFI loader + popcorn-uefi.img
./scripts/core.sh run-uefi   # interactive QEMU (UEFI USB image)
./scripts/core.sh clean      # wipe target/
```

Platform wrappers (dialog menu when run with no args):

| Script | Platform |
|--------|----------|
| `scripts/macos.sh` | macOS |
| `scripts/fedora.sh` | Fedora / RHEL |
| `scripts/linux.sh` | Generic Linux (Debian/Ubuntu/Arch/…) |

### GUI builders

| Script | Platform |
|--------|----------|
| `python3 scripts/gui-macos.py` | macOS (pywebview) |
| `python3 scripts/gui-fedora.py` | Fedora (Tkinter) |
| `python3 scripts/gui-linux.py` | Linux (Tkinter) |
| `python scripts/gui-win.py` | Windows via WSL2 (Tkinter) |

On Windows, use `gui-win.py` or `scripts/win.ps1`. Both go through WSL, build the **hardware-safe** `target/popcorn-uefi.img` (no auto-install kernel), and run QEMU with **only files under `target/`** — never `PhysicalDrive` / the Windows NVMe.

```powershell
python scripts/gui-win.py
powershell -File scripts/win.ps1 all
powershell -File scripts/win.ps1 run-uefi
```

Or inside WSL:

```bash
wsl
cd /mnt/c/Users/<you>/Documents/Code/popcorn
./scripts/core.sh all && ./scripts/core.sh run-uefi
```

Do not flash `target/UNSAFE/*`. Do not run `test-install` from Windows.

### Try the Features

- `help` — available commands
- `sysinfo` — system information
- `mem -map` — memory layout
- `dol -new test.txt` — create a text file
- `ls` — list files

## Layout

```
popcorn/
├── scripts/           # build & GUI entry points
│   ├── core.sh        # unified CLI
│   ├── macos.sh / fedora.sh / linux.sh
│   ├── gui-*.py
│   ├── lib/           # shell modules
│   └── popcorn_build/ # Python build library
├── src/               # kernel sources only
│   ├── core/ pops/ includes/ uefi/
│   ├── rust/          # no_std crate (libpopcorn_kernel.a)
│   └── link.ld
└── target/            # ALL build outputs (gitignored)
    ├── kernel
    ├── BOOTX64.EFI
    ├── popcorn-uefi.img   # ONLY flash this
    └── UNSAFE/            # QEMU-only; never flash
```

## Dependencies

**Fedora/RHEL:**
```bash
sudo dnf install nasm clang lld qemu-system-x86 grub2-tools-extra grub2-pc-modules xorriso mtools edk2-ovmf dosfstools
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-none
```

**Ubuntu/Debian:**
```bash
sudo apt install nasm clang lld qemu-system-x86 grub-pc-bin grub-common xorriso mtools ovmf dosfstools
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-none
```

**macOS (Homebrew):**
```bash
brew install nasm qemu xorriso mtools llvm i686-elf-grub x86_64-elf-grub x86_64-elf-binutils x86_64-elf-gcc
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-none
```

## Architecture

Popcorn is a 64-bit (x86-64) kernel with long mode, Multiboot2 + native UEFI boot, an in-kernel console, and modular “pops”.

See `roadmap.md` for the Rust driver / kernel maturity plan, and `pop.md` for writing pops.
