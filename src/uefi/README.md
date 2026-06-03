Native UEFI test loader
=======================

This path bypasses GRUB/Multiboot and boots directly as `BOOTX64.EFI`.

Build (from `src/`):

```bash
./build/core.sh all          # kernel + BOOTX64.EFI + popcorn-uefi.img
./build/core.sh test-uefi    # QEMU smoke (stability + alive + debugcon)
```

Artifacts:

- `BOOTX64.EFI`
- `popcorn-uefi.img` — flash to USB (Balena Etcher)
- `uefi_usb/EFI/BOOT/BOOTX64.EFI` — manual FAT32 copy layout

Hardware:

1. Flash `popcorn-uefi.img` to a USB stick, or format FAT32 and copy `uefi_usb/EFI/BOOT/BOOTX64.EFI` plus `boot/kernel`.
2. Boot the USB in UEFI mode.

QEMU (interactive):

```bash
./build/core.sh run-uefi
```
