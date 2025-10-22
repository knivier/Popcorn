// src/includes/multiboot2.h
#ifndef MULTIBOOT2_H
#define MULTIBOOT2_H

#include <stdint.h>
#include <stdbool.h>

// Multiboot2 specification structures
// Reference: https://www.gnu.org/software/grub/manual/multiboot2/multiboot.html

#define MULTIBOOT2_BOOTLOADER_MAGIC 0x36d76289

// Multiboot2 tag types
#define MULTIBOOT_TAG_TYPE_END               0
#define MULTIBOOT_TAG_TYPE_CMDLINE           1
#define MULTIBOOT_TAG_TYPE_BOOT_LOADER_NAME  2
#define MULTIBOOT_TAG_TYPE_MODULE            3
#define MULTIBOOT_TAG_TYPE_BASIC_MEMINFO     4
#define MULTIBOOT_TAG_TYPE_BOOTDEV           5
#define MULTIBOOT_TAG_TYPE_MMAP              6
#define MULTIBOOT_TAG_TYPE_VBE               7
#define MULTIBOOT_TAG_TYPE_FRAMEBUFFER       8
#define MULTIBOOT_TAG_TYPE_ELF_SECTIONS      9
#define MULTIBOOT_TAG_TYPE_APM               10
#define MULTIBOOT_TAG_TYPE_EFI32             11
#define MULTIBOOT_TAG_TYPE_EFI64             12
#define MULTIBOOT_TAG_TYPE_SMBIOS            13
#define MULTIBOOT_TAG_TYPE_ACPI_OLD          14
#define MULTIBOOT_TAG_TYPE_ACPI_NEW          15
#define MULTIBOOT_TAG_TYPE_NETWORK           16
#define MULTIBOOT_TAG_TYPE_EFI_MMAP          17
#define MULTIBOOT_TAG_TYPE_EFI_BS            18

// Base tag structure
struct multiboot_tag {
    uint32_t type;
    uint32_t size;
};

// Boot loader name tag
struct multiboot_tag_string {
    uint32_t type;
    uint32_t size;
    char string[0];
};

// Basic memory info tag
struct multiboot_tag_basic_meminfo {
    uint32_t type;
    uint32_t size;
    uint32_t mem_lower;  // KB of lower memory (0-640KB)
    uint32_t mem_upper;  // KB of upper memory
};

// Memory map entry
struct multiboot_mmap_entry {
    uint64_t addr;
    uint64_t len;
    uint32_t type;
    uint32_t zero;
} __attribute__((packed));

// Memory map tag
struct multiboot_tag_mmap {
    uint32_t type;
    uint32_t size;
    uint32_t entry_size;
    uint32_t entry_version;
    struct multiboot_mmap_entry entries[0];
};

// Memory map entry types
#define MULTIBOOT_MEMORY_AVAILABLE        1
#define MULTIBOOT_MEMORY_RESERVED         2
#define MULTIBOOT_MEMORY_ACPI_RECLAIMABLE 3
#define MULTIBOOT_MEMORY_NVS              4
#define MULTIBOOT_MEMORY_BADRAM           5

// Parsed system information
typedef struct {
    bool valid;
    char bootloader_name[64];
    char command_line[128];
    uint32_t mem_lower;      // KB
    uint32_t mem_upper;      // KB
    uint64_t total_memory;   // Bytes (calculated from memory map)
    uint32_t available_memory_regions;
} SystemInfo;

// External reference to multiboot info pointer (set in core/kernel.asm)
extern uint64_t multiboot2_info_ptr;

// Function declarations
void multiboot2_parse(void);
SystemInfo* multiboot2_get_info(void);
const char* multiboot2_get_bootloader_name(void);
const char* multiboot2_get_command_line(void);
uint64_t multiboot2_get_total_memory(void);
uint32_t multiboot2_get_memory_lower(void);
uint32_t multiboot2_get_memory_upper(void);

#endif // MULTIBOOT2_H

