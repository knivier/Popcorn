#ifndef UEFI_BOOT_H
#define UEFI_BOOT_H

#include <stdint.h>

#define POPCORN_UEFI_MAGIC        0x504F50434F524E42ULL /* "POPCORNB" */
/* Fixed .bss.boot layout (kernel.asm); must match loader handoff write address. */
#define POPCORN_UEFI_MBI_PHYS     0x102000ULL
#define POPCORN_UEFI_HANDOFF_PHYS 0x102008ULL
#define POPCORN_UEFI_FLAG_FIRMWARE_KBD 0x01u /* BootServices left up; use conin */
#define POPCORN_UEFI_FLAG_INPUT_EX     0x02u /* conin is SimpleTextInputEx* */

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
} PopcornUefiBootInfo;

extern volatile PopcornUefiBootInfo popcorn_uefi_handoff;

#endif
