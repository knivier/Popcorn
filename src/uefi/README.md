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
- `target/popcorn-uefi.img` — flash to USB (Balena Etcher)
- `target/uefi_usb/EFI/BOOT/BOOTX64.EFI` — manual FAT32 copy layout

Hardware:

1. Flash `target/popcorn-uefi.img` to a USB stick, or format FAT32 and copy the ESP layout from `target/uefi_usb/` plus kernel.
2. Boot the USB in UEFI mode.

QEMU (interactive):

```bash
./scripts/core.sh run-uefi
```
