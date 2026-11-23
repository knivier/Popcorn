#ifndef UEFI_BOOT_H
#define UEFI_BOOT_H

#include <stdint.h>

#define POPCORN_UEFI_MAGIC        0x504F50434F524E42ULL /* "POPCORNB" */
/* Fixed .bss.boot layout (kernel.asm); must match loader handoff write address.
 * link.ld ASSERTs that the popcorn_uefi_handoff symbol really lands here. */
#define POPCORN_UEFI_HANDOFF_PHYS 0x102008ULL
/* Bytes reserved for the handoff in kernel.asm (.bss.boot `resb`). */
#define POPCORN_UEFI_HANDOFF_SIZE 96u
/* Byte offset of mbi_phys; kernel.asm reads it directly (POPCORN_HANDOFF_MBI_OFF). */
#define POPCORN_UEFI_HANDOFF_MBI_OFF 72u
#define POPCORN_UEFI_FLAG_FIRMWARE_KBD 0x01u /* BootServices left up; use conin */
#define POPCORN_UEFI_FLAG_INPUT_EX     0x02u /* conin is SimpleTextInputEx* */
#define POPCORN_UEFI_BOOT_VOL_VALID    0x564F4C42u /* "BLOV" */

typedef struct {
    uint64_t magic;
    uint64_t framebuffer_addr;
    uint32_t framebuffer_pitch;
    uint32_t framebuffer_width;
    uint32_t framebuffer_height;
    uint8_t  framebuffer_bpp;
    uint8_t  framebuffer_type; /* 1 = RGB linear */
    uint8_t  red_pos;
    uint8_t  red_size;
    uint8_t  green_pos;
    uint8_t  green_size;
    uint8_t  blue_pos;
    uint8_t  blue_size;
    uint8_t  flags;
    uint8_t  reserved[3];
    uint64_t conin;           /* TextIn or TextInEx protocol pointer */
    uint64_t boot_services;   /* EFI_BOOT_SERVICES* */
    uint64_t firmware_cr3;    /* CR3 while firmware still owns USB (before kernel switch) */
    uint64_t available_ram_bytes; /* conventional RAM above 1 MiB (informational; the MBI mmap is authoritative) */
    uint64_t mbi_phys;        /* loader-built Multiboot2 MBI (AllocatePages, LoaderData, < 4 GiB) */
    /* The FAT volume BOOTX64.EFI was loaded from (its boot sector, read through
     * BlockIo on the LoadedImage DeviceHandle), so the kernel can tell which
     * disk it booted from instead of guessing. Valid when boot_vol_valid ==
     * POPCORN_UEFI_BOOT_VOL_VALID. */
    uint32_t boot_vol_serial;  /* BPB volume serial (FAT32 off 67, FAT12/16 off 39) */
    uint32_t boot_vol_valid;
    uint64_t boot_vol_sectors; /* BPB total sectors */
} PopcornUefiBootInfo;

_Static_assert(sizeof(PopcornUefiBootInfo) <= POPCORN_UEFI_HANDOFF_SIZE,
               "PopcornUefiBootInfo outgrew the kernel.asm handoff slot");
_Static_assert(__builtin_offsetof(PopcornUefiBootInfo, mbi_phys) == POPCORN_UEFI_HANDOFF_MBI_OFF,
               "mbi_phys offset must match kernel.asm POPCORN_HANDOFF_MBI_OFF");

extern volatile PopcornUefiBootInfo popcorn_uefi_handoff;

#endif
