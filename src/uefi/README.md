Native UEFI loader
==================

This path bypasses GRUB/Multiboot and boots directly as `BOOTX64.EFI`.

Build (from repo root):

```bash
./scripts/core.sh all          # kernel + BOOTX64.EFI + popcorn-uefi.img → target/
./scripts/core.sh test-uefi    # QEMU smoke (stability + alive + debugcon + #PF dump + GRUB ISO)
```

Artifacts (all under `target/`):

- `target/BOOTX64.EFI` — the loader alone; it cannot boot without the kernel next to it
- `target/popcorn-uefi.img` — **the only file to flash** (Balena Etcher)
- `target/UNSAFE/` — QEMU scratch + auto-install test images. **Do not flash.**

A hand-made FAT32 stick needs the loader at `\EFI\BOOT\BOOTX64.EFI` and the
kernel at `\boot\kernel` (or `\EFI\BOOT\kernel`) on the same volume; flashing
`popcorn-uefi.img` gives exactly that layout.

Hardware:

1. Flash **only** `target/popcorn-uefi.img` (built with `./scripts/core.sh all`, never `test-install`).
2. Boot the USB in UEFI mode. The shell must come up with no install. Use `disk install usb0 YES` to format a USB stick. NVMe writes stay off until `disk master nvme0 YES`.

QEMU (interactive):

```bash
./scripts/core.sh run-uefi
```

Windows (WSL, hardware-safe image only; QEMU never attaches the Windows disk):

```powershell
powershell -File scripts/win.ps1 all
powershell -File scripts/win.ps1 run-uefi
```
