// src/includes/multiboot2.h
#ifndef MULTIBOOT2_H
#define MULTIBOOT2_H

#include <stdint.h>
#include <stdbool.h>

// Multiboot2 specification structures
// Reference: https://www.gnu.org/software/grub/manual/multiboot2/multiboot.html

#define MULTIBOOT2_BOOTLOADER_MAGIC 0x36d76289
#define MBI_STAGING_SIZE              65536

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

/* VBE mode info (subset used by Multiboot2 VBE tag). */
struct multiboot_vbe_mode_info {
    uint16_t attributes;
    uint8_t  window_a;
    uint8_t  window_b;
    uint16_t granularity;
    uint16_t window_size;
    uint16_t segment_a;
    uint16_t segment_b;
    uint32_t win_func_ptr;
    uint16_t pitch;
    uint16_t width;
    uint16_t height;
    uint8_t  w_char;
    uint8_t  y_char;
    uint8_t  planes;
    uint8_t  bpp;
    uint8_t  banks;
    uint8_t  memory_model;
    uint8_t  bank_size;
    uint8_t  image_pages;
    uint8_t  reserved0;
    uint8_t  red_mask;
    uint8_t  red_position;
    uint8_t  green_mask;
    uint8_t  green_position;
    uint8_t  blue_mask;
    uint8_t  blue_position;
    uint8_t  reserved_mask;
    uint8_t  reserved_position;
    uint8_t  direct_color_attributes;
    uint32_t framebuffer;
    uint32_t off_screen_mem_off;
    uint16_t off_screen_mem_size;
    uint8_t  reserved1[206];
} __attribute__((packed));

// VBE tag (type 7)
struct multiboot_tag_vbe {
    uint32_t type;
    uint32_t size;
    uint16_t vbe_mode;
    uint16_t vbe_interface_seg;
    uint16_t vbe_interface_off;
    uint16_t vbe_interface_len;
    uint8_t  vbe_control_info[512];
    struct multiboot_vbe_mode_info mode_info;
} __attribute__((packed));

// Framebuffer tag (type 8)
struct multiboot_tag_framebuffer {
    uint32_t type;
    uint32_t size;
    uint64_t framebuffer_addr;
    uint32_t framebuffer_pitch;
    uint32_t framebuffer_width;
    uint32_t framebuffer_height;
    uint8_t  framebuffer_bpp;
    uint8_t  framebuffer_type;
    uint16_t reserved;
    uint8_t  framebuffer_red_field_position;
    uint8_t  framebuffer_red_mask_size;
    uint8_t  framebuffer_green_field_position;
    uint8_t  framebuffer_green_mask_size;
    uint8_t  framebuffer_blue_field_position;
    uint8_t  framebuffer_blue_mask_size;
} __attribute__((packed));

#define MULTIBOOT_FRAMEBUFFER_TYPE_INDEXED  0
#define MULTIBOOT_FRAMEBUFFER_TYPE_RGB      1
#define MULTIBOOT_FRAMEBUFFER_TYPE_EGA_TEXT 2

// Parsed framebuffer info (valid only when GRUB provided a framebuffer tag)
typedef struct {
    bool present;
    uint64_t addr;
    uint32_t pitch;
    uint32_t width;
    uint32_t height;
    uint8_t  bpp;
    uint8_t  type;
    uint8_t  red_pos, red_size;
    uint8_t  green_pos, green_size;
    uint8_t  blue_pos, blue_size;
} FramebufferInfo;

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

/* Iterates MMAP tag entries; safe after multiboot2_parse. */
typedef void (*multiboot_mmap_fn)(uint64_t base, uint64_t len, uint32_t type, void* user);
void multiboot2_foreach_mmap(multiboot_mmap_fn fn, void* user);

// Function declarations
void multiboot2_parse(void);
/* Walk tags for framebuffer/VBE only — call before console_init on UEFI hardware. */
void multiboot2_parse_framebuffer(void);
void popcorn_apply_uefi_boot_info(void);
void uefi_boot_mark_console(void);
bool multiboot2_is_uefi_boot(void);
SystemInfo* multiboot2_get_info(void);
const FramebufferInfo* multiboot2_get_framebuffer(void);
const char* multiboot2_get_bootloader_name(void);
const char* multiboot2_get_command_line(void);
uint64_t multiboot2_get_total_memory(void);
uint32_t multiboot2_get_memory_lower(void);
uint32_t multiboot2_get_memory_upper(void);

#endif // MULTIBOOT2_H

