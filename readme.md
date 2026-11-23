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

Platform wrappers (thin stubs over `scripts/lib/wrapper.sh`; arguments are passed to `core.sh`, and a bare run opens a `dialog` menu, or prints `core.sh help` if `dialog` is not installed):

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

### Boot log on real hardware (`POPCORN.LOG`)

Most laptops have no serial port, so Popcorn keeps its own log on the USB stick
it booted from. Flash `target/popcorn-uefi.img`, boot the machine from the stick,
then plug the stick into another computer and open `POPCORN.LOG` in its root. It
holds the last boot: every console line plus the driver diagnostics that normally
go only to COM1 (PCI list, `net:`, `virtio-blk:`, Rust panics, CPU exception
dumps), each with a timestamp in seconds since boot. The first line says which
boot it is (`boot N`, counting up on every boot from that stick) and how many
bytes are used; the rest of the 1 MiB file is blank padding.

It is written right after the disks come up, after the drivers probe, about once a
second while there is new text, on `log -flush` and `stop`, and on a panic or CPU
exception. So pull the stick after the machine hangs or halts, or after `stop`.
`log` shows where it goes, and `log -verify` reads it back from the stick. The
kernel only rewrites sectors inside that file (never the FAT or directories), and
stops writing if the stick is opened for `disk -install` / `disk -wipe`. QEMU runs
open the boot stick with `snapshot=on`, so they never change the flashable image.

### Try the Features

- `help` — available commands
- `sysinfo` — system information
- `mem -map` — memory layout
- `disk -list` / `disk -wipe <name> YES` — pick and FAT32-format a data disk (QEMU data disks start blank; nothing is auto-formatted)
- `dol -new test.txt` — open the editor on a new file (`dol -save` writes it to the FAT32 disk)
- `ls` — list files on the mounted FAT32 disk

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
│   ├── core/ includes/ uefi/
│   ├── rust/          # no_std crate (libpopcorn_kernel.a): drivers, FAT32, all pops (popcorn_kernel/src/pops/)
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
brew install nasm qemu xorriso mtools llvm lld i686-elf-grub x86_64-elf-grub x86_64-elf-binutils x86_64-elf-gcc
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-none
```

## Architecture

Popcorn is a 64-bit (x86-64) kernel with long mode, Multiboot2 + native UEFI boot, an in-kernel console, and modular “pops”.

See `roadmap.md` for the Rust driver / kernel maturity plan, and `pop.md` for writing pops.
