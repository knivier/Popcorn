Native UEFI test loader
=======================

This path bypasses GRUB/Multiboot and boots directly as `BOOTX64.EFI`.

Build on macOS/Linux (with clang + lld-link):

```bash
bash build/uefi.sh
```

Artifacts:

- `BOOTX64.EFI`
- `uefi_usb/EFI/BOOT/BOOTX64.EFI`

To test on hardware:

1. Format a USB stick as FAT32.
2. Copy `uefi_usb/EFI/BOOT/BOOTX64.EFI` onto it.
3. Boot the USB in UEFI mode.

Expected behavior: firmware text output saying `Popcorn Native UEFI Loader`.

### QEMU test (macOS Homebrew qemu)

```bash
bash build/run-uefi-qemu.sh
```

This boots `uefi_usb/EFI/BOOT/BOOTX64.EFI` under OVMF and saves a screendump to
`buildbase/uefi-screen.ppm` for automated verification.

For a visible window, run the interactive command printed at the end of the script.
