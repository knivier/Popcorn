// src/core/multiboot2.c
#include "../includes/multiboot2.h"
#include "../includes/uefi_boot.h"
#include "../includes/boot_fb.h"
#include "../includes/console.h"
#include <stddef.h>
#include <stdint.h>

extern uint64_t multiboot2_info_ptr;

// Static storage for parsed system info
static SystemInfo sys_info = {0};

// Static storage for parsed framebuffer info (filled if GRUB provides a tag)
static FramebufferInfo fb_info = {0};

// String copy helper
static void strncpy_safe(char* dest, const char* src, size_t max_len) {
    size_t i;
    for (i = 0; i < max_len - 1 && src[i] != '\0'; i++) {
        dest[i] = src[i];
    }
    dest[i] = '\0';
}

static void fb_info_from_framebuffer_tag(const struct multiboot_tag_framebuffer* fb) {
    fb_info.present = true;
    fb_info.addr = fb->framebuffer_addr;
    fb_info.pitch = fb->framebuffer_pitch;
    fb_info.width = fb->framebuffer_width;
    fb_info.height = fb->framebuffer_height;
    fb_info.bpp = fb->framebuffer_bpp;
    fb_info.type = fb->framebuffer_type;
    fb_info.red_pos = fb->framebuffer_red_field_position;
    fb_info.red_size = fb->framebuffer_red_mask_size;
    fb_info.green_pos = fb->framebuffer_green_field_position;
    fb_info.green_size = fb->framebuffer_green_mask_size;
    fb_info.blue_pos = fb->framebuffer_blue_field_position;
    fb_info.blue_size = fb->framebuffer_blue_mask_size;
}

static void fb_info_from_vbe_tag(const struct multiboot_tag_vbe* vbe) {
    const struct multiboot_vbe_mode_info* mode = &vbe->mode_info;
    if (mode->framebuffer == 0 || mode->width == 0 || mode->height == 0) {
        return;
    }
    fb_info.present = true;
    fb_info.addr = mode->framebuffer;
    fb_info.pitch = mode->pitch;
    fb_info.width = mode->width;
    fb_info.height = mode->height;
    fb_info.bpp = mode->bpp;
    fb_info.type = MULTIBOOT_FRAMEBUFFER_TYPE_RGB;
    fb_info.red_pos = mode->red_position;
    fb_info.red_size = mode->red_mask ? mode->red_mask : 8;
    fb_info.green_pos = mode->green_position;
    fb_info.green_size = mode->green_mask ? mode->green_mask : 8;
    fb_info.blue_pos = mode->blue_position;
    fb_info.blue_size = mode->blue_mask ? mode->blue_mask : 8;
}

static void multiboot2_walk_tags(bool framebuffer_only) {
    FramebufferInfo tag8 = {0};
    FramebufferInfo tag7 = {0};
    fb_info.present = false;
    if (multiboot2_info_ptr == 0) {
        boot_serial_putc('M');
        return;
    }
    uint32_t* mbi = (uint32_t*)(uintptr_t)multiboot2_info_ptr;
    uint32_t total_size = mbi[0];
    if (total_size < 8 || total_size > 0x100000) {
        boot_serial_putc('?');
        return;
    }
    struct multiboot_tag* tag = (struct multiboot_tag*)((uint8_t*)mbi + 8);
    while (tag->type != MULTIBOOT_TAG_TYPE_END) {
        if (tag->type == MULTIBOOT_TAG_TYPE_FRAMEBUFFER &&
            tag->size >= sizeof(struct multiboot_tag_framebuffer)) {
            fb_info_from_framebuffer_tag((const struct multiboot_tag_framebuffer*)tag);
            tag8 = fb_info;
        } else if (tag->type == MULTIBOOT_TAG_TYPE_VBE &&
                   tag->size >= sizeof(struct multiboot_tag_vbe)) {
            FramebufferInfo saved = fb_info;
            fb_info_from_vbe_tag((const struct multiboot_tag_vbe*)tag);
            tag7 = fb_info;
            fb_info = saved;
        }
        if (!framebuffer_only) {
            switch (tag->type) {
                case MULTIBOOT_TAG_TYPE_BOOT_LOADER_NAME: {
                    struct multiboot_tag_string* name_tag = (struct multiboot_tag_string*)tag;
                    strncpy_safe(sys_info.bootloader_name, name_tag->string, sizeof(sys_info.bootloader_name));
                    break;
                }
                case MULTIBOOT_TAG_TYPE_CMDLINE: {
                    struct multiboot_tag_string* cmd_tag = (struct multiboot_tag_string*)tag;
                    strncpy_safe(sys_info.command_line, cmd_tag->string, sizeof(sys_info.command_line));
                    break;
                }
                case MULTIBOOT_TAG_TYPE_BASIC_MEMINFO: {
                    struct multiboot_tag_basic_meminfo* mem_tag = (struct multiboot_tag_basic_meminfo*)tag;
                    sys_info.mem_lower = mem_tag->mem_lower;
                    sys_info.mem_upper = mem_tag->mem_upper;
                    break;
                }
                case MULTIBOOT_TAG_TYPE_MMAP: {
                    struct multiboot_tag_mmap* mmap_tag = (struct multiboot_tag_mmap*)tag;
                    uint32_t num_entries = (mmap_tag->size - sizeof(struct multiboot_tag_mmap)) / mmap_tag->entry_size;
                    for (uint32_t i = 0; i < num_entries; i++) {
                        struct multiboot_mmap_entry* entry = (struct multiboot_mmap_entry*)
                            ((uint8_t*)mmap_tag->entries + i * mmap_tag->entry_size);
                        if (entry->type == MULTIBOOT_MEMORY_AVAILABLE) {
                            sys_info.total_memory += entry->len;
                            sys_info.available_memory_regions++;
                        }
                    }
                    break;
                }
                default:
                    break;
            }
        }
        uint32_t tag_size = tag->size;
        tag = (struct multiboot_tag*)((uint8_t*)tag + ((tag_size + 7) & ~7));
        if ((uintptr_t)tag >= (uintptr_t)mbi + total_size) {
            break;
        }
    }
    if (tag8.present) {
        fb_info = tag8;
    } else if (tag7.present) {
        fb_info = tag7;
    } else {
        fb_info.present = false;
    }
    if (framebuffer_only) {
        boot_serial_putc(fb_info.present ? 'F' : 'V');
    }
}

void multiboot2_parse_framebuffer(void) {
    popcorn_apply_uefi_boot_info();
    if (!fb_info.present) {
        multiboot2_walk_tags(true);
    }
}

static volatile PopcornUefiBootInfo* uefi_handoff_ptr(void) {
    return (volatile PopcornUefiBootInfo*)(uintptr_t)POPCORN_UEFI_HANDOFF_PHYS;
}

bool multiboot2_is_uefi_boot(void) {
    volatile PopcornUefiBootInfo* u = uefi_handoff_ptr();
    return u->magic == POPCORN_UEFI_MAGIC;
}

void uefi_boot_mark_console(void) {
    popcorn_apply_uefi_boot_info();
}

void popcorn_apply_uefi_boot_info(void) {
    volatile PopcornUefiBootInfo* u = uefi_handoff_ptr();
    if (u->magic != POPCORN_UEFI_MAGIC) {
        return;
    }

    fb_info.present = true;
    fb_info.addr = u->framebuffer_addr;
    fb_info.pitch = u->framebuffer_pitch;
    fb_info.width = u->framebuffer_width;
    fb_info.height = u->framebuffer_height;
    fb_info.bpp = u->framebuffer_bpp;
    fb_info.type = u->framebuffer_type;
    fb_info.red_pos = u->red_pos;
    fb_info.red_size = u->red_size ? u->red_size : 8;
    fb_info.green_pos = u->green_pos;
    fb_info.green_size = u->green_size ? u->green_size : 8;
    fb_info.blue_pos = u->blue_pos;
    fb_info.blue_size = u->blue_size ? u->blue_size : 8;
}

/*
 * Parse Multiboot2 information structure
 * Called during kernel initialization
 */
void multiboot2_parse(void) {
    // Initialize system info
    sys_info.valid = false;
    sys_info.bootloader_name[0] = '\0';
    sys_info.command_line[0] = '\0';
    sys_info.mem_lower = 0;
    sys_info.mem_upper = 0;
    sys_info.total_memory = 0;
    sys_info.available_memory_regions = 0;

    if (multiboot2_info_ptr == 0 && multiboot2_is_uefi_boot()) {
        volatile PopcornUefiBootInfo* u = uefi_handoff_ptr();
        strncpy_safe(sys_info.bootloader_name, "Popcorn UEFI", sizeof(sys_info.bootloader_name));
        sys_info.mem_lower = 640;
        sys_info.mem_upper = (uint32_t)(u->available_ram_bytes / 1024ULL);
        sys_info.total_memory = u->available_ram_bytes;
        sys_info.valid = true;
        multiboot2_info_ptr = POPCORN_UEFI_MBI_PHYS;
    }

    if (multiboot2_info_ptr == 0) {
        sys_info.mem_lower = 640;
        sys_info.mem_upper = 0;
        sys_info.valid = true;
        return;
    }
    boot_serial_putc('P');
    multiboot2_walk_tags(false);
    sys_info.valid = true;
    if (sys_info.total_memory > 0) {
        boot_serial_putc('R');
    }
}

void multiboot2_foreach_mmap(multiboot_mmap_fn fn, void* user) {
    if (!fn) {
        return;
    }
    if (multiboot2_info_ptr == 0) {
        return;
    }
    uint32_t* mbi = (uint32_t*)(uintptr_t)multiboot2_info_ptr;
    uint32_t total_size = mbi[0];
    if (total_size < 8 || total_size > 0x100000) {
        return;
    }
    struct multiboot_tag* tag = (struct multiboot_tag*)((uint8_t*)mbi + 8);
    while (tag->type != MULTIBOOT_TAG_TYPE_END) {
        if (tag->type == MULTIBOOT_TAG_TYPE_MMAP) {
            struct multiboot_tag_mmap* mmap_tag = (struct multiboot_tag_mmap*)tag;
            uint32_t num_entries = (mmap_tag->size - sizeof(struct multiboot_tag_mmap)) / mmap_tag->entry_size;
            for (uint32_t i = 0; i < num_entries; i++) {
                struct multiboot_mmap_entry* entry = (struct multiboot_mmap_entry*)
                    ((uint8_t*)mmap_tag->entries + i * mmap_tag->entry_size);
                fn(entry->addr, entry->len, entry->type, user);
            }
        }
        uint32_t tag_size = tag->size;
        tag = (struct multiboot_tag*)((uint8_t*)tag + ((tag_size + 7) & ~7U));
        if ((uintptr_t)tag >= (uintptr_t)mbi + total_size) {
            break;
        }
    }
}

/*
 * Get pointer to parsed system info
 */
SystemInfo* multiboot2_get_info(void) {
    return &sys_info;
}

/*
 * Get parsed framebuffer info (present=false if GRUB gave no framebuffer tag)
 */
const FramebufferInfo* multiboot2_get_framebuffer(void) {
    return &fb_info;
}

/*
 * Get bootloader name
 */
const char* multiboot2_get_bootloader_name(void) {
    if (!sys_info.valid || sys_info.bootloader_name[0] == '\0') {
        return "Unknown";
    }
    return sys_info.bootloader_name;
}

/*
 * Get kernel command line
 */
const char* multiboot2_get_command_line(void) {
    if (!sys_info.valid || sys_info.command_line[0] == '\0') {
        return "";
    }
    return sys_info.command_line;
}

/*
 * Get total memory in bytes
 */
uint64_t multiboot2_get_total_memory(void) {
    if (!sys_info.valid || sys_info.total_memory == 0) {
        // mem_upper is in KB, starting from 1MB
        if (sys_info.mem_upper > 0) {
            return (1024ULL * 1024) + ((uint64_t)sys_info.mem_upper * 1024);
        }
        // If still nothing, return a minimal amount
        return 1024ULL * 1024;  // 1MB minimum
    }
    return sys_info.total_memory;
}

/*
 * Get lower memory in KB
 */
uint32_t multiboot2_get_memory_lower(void) {
    return sys_info.mem_lower;
}

/*
 * Get upper memory in KB
 */
uint32_t multiboot2_get_memory_upper(void) {
    return sys_info.mem_upper;
}

/* ---- boot_fb (linked via multiboot2.o so all build scripts pick it up) ---- */

extern uint64_t page_table_l2[];
extern uint64_t page_table_l3[];

#define IDENTITY_2M_PAGES (64u * 512u)
#define IDENTITY_1G_PAGES 512u

void boot_serial_putc(char c) {
    /* QEMU -debugcon (isa-debugcon iobase 0xe9); harmless on bare metal. */
    __asm__ volatile("outb %0, $0xE9" : : "a"(c) : "memory");
    for (int i = 0; i < 100000; i++) {
        unsigned char s;
        __asm__ volatile("inb %1, %0" : "=a"(s) : "Nd"((unsigned short)0x3FD));
        if (s & 0x20) {
            break;
        }
    }
    __asm__ volatile("outb %0, %1" : : "a"(c), "Nd"((unsigned short)0x3F8));
}

void boot_identity_uncache_range(uint64_t phys, uint64_t len) {
    if (len == 0) {
        return;
    }

    /* GRUB path: low identity uses 2 MiB pages in page_table_l2. */
    uint64_t start2m = phys & ~((1ULL << 21) - 1ULL);
    uint64_t end = phys + len;
    for (uint64_t a = start2m; a < end; a += (1ULL << 21)) {
        uint32_t idx = (uint32_t)(a >> 21);
        if (idx >= IDENTITY_2M_PAGES) {
            break;
        }
        /* Only a real 2 MiB page (PS set) is a leaf we should retype here. */
        if (page_table_l2[idx] & 0x80ULL) {
            page_table_l2[idx] = (page_table_l2[idx] & ~0x18ULL) | 0x18ULL;
            __asm__ volatile("invlpg (%0)" : : "r"((uintptr_t)a) : "memory");
        }
    }

    /* UEFI path: low identity uses 1 GiB pages in page_table_l3. Mark the
     * covering 1 GiB page(s) uncacheable (PCD|PWT) so MMIO framebuffer writes
     * are not absorbed by the cache on real hardware. */
    uint64_t start1g = phys & ~((1ULL << 30) - 1ULL);
    for (uint64_t a = start1g; a < end; a += (1ULL << 30)) {
        uint32_t idx = (uint32_t)(a >> 30);
        if (idx >= IDENTITY_1G_PAGES) {
            break;
        }
        if (page_table_l3[idx] & 0x80ULL) {
            page_table_l3[idx] = (page_table_l3[idx] & ~0x18ULL) | 0x18ULL;
            __asm__ volatile("invlpg (%0)" : : "r"((uintptr_t)a) : "memory");
        }
    }
}

void boot_fb_solid_fill(volatile uint8_t* base, uint32_t pitch, uint32_t width,
                        uint32_t height, uint8_t bytespp, uint32_t rgb) {
    if (!base || pitch == 0 || width == 0 || height == 0 || bytespp < 3) {
        return;
    }
    uint8_t b = (uint8_t)(rgb & 0xFFu);
    uint8_t g = (uint8_t)((rgb >> 8) & 0xFFu);
    uint8_t r = (uint8_t)((rgb >> 16) & 0xFFu);
    if (bytespp == 4 && pitch >= width * 4u) {
        uint32_t pix = (uint32_t)b | ((uint32_t)g << 8) | ((uint32_t)r << 16);
        for (uint32_t y = 0; y < height; y++) {
            volatile uint32_t* row =
                (volatile uint32_t*)(base + (uint64_t)y * pitch);
            for (uint32_t x = 0; x < width; x++) {
                row[x] = pix;
            }
        }
        return;
    }
    for (uint32_t y = 0; y < height; y++) {
        volatile uint8_t* row = base + (uint64_t)y * pitch;
        for (uint32_t x = 0; x < width; x++) {
            volatile uint8_t* px = row + (uint64_t)x * bytespp;
            px[0] = b;
            px[1] = g;
            px[2] = r;
            if (bytespp >= 4) {
                px[3] = 0;
            }
        }
    }
}

static void kbc_wait_input(void) {
    for (int i = 0; i < 100000; i++) {
        unsigned char s;
        __asm__ volatile("inb %1, %0" : "=a"(s) : "Nd"((unsigned short)0x64));
        if ((s & 0x02) == 0) {
            return;
        }
    }
}

void boot_diag_caps_led_on(void) {
    kbc_wait_input();
    __asm__ volatile("outb %0, %1" : : "a"((unsigned char)0xED), "Nd"((unsigned short)0x60));
    kbc_wait_input();
    __asm__ volatile("outb %0, %1" : : "a"((unsigned char)0x02), "Nd"((unsigned short)0x60));
}

