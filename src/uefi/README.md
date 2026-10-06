Native UEFI test loader
=======================

This path bypasses GRUB/Multiboot and boots directly as `BOOTX64.EFI`.

Build (from repo root):

```bash
./scripts/core.sh all          # kernel + BOOTX64.EFI + popcorn-uefi.img → target/
./scripts/core.sh test-uefi    # QEMU smoke (stability + alive + debugcon)
```

Artifacts (all under `target/`):

- `target/BOOTX64.EFI`
- `target/popcorn-uefi.img` — **the only file to flash** (Balena Etcher)
- `target/UNSAFE/` — QEMU scratch + auto-install test images. **Do not flash.**
- `target/uefi_usb/EFI/BOOT/BOOTX64.EFI` — manual FAT32 copy layout

Hardware:

1. Flash **only** `target/popcorn-uefi.img` (built with `./scripts/core.sh all`, never `test-install`).
2. Boot the USB in UEFI mode. The shell must come up with no install. Use `disk install usb0 YES` to format a USB stick. NVMe writes stay off until `disk master nvme0 YES`.

QEMU (interactive):

```bash
./scripts/core.sh run-uefi
```
