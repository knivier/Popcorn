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

/* MBI bounds [first tag, end), or NULL if the pointer or total_size is unusable. */
static uint8_t* mbi_first_tag(uint8_t** end_out) {
    if (multiboot2_info_ptr == 0) {
        return NULL;
    }
    uint8_t* mbi = (uint8_t*)(uintptr_t)multiboot2_info_ptr;
    uint32_t total_size = *(uint32_t*)mbi;
    if (total_size < 8 || total_size > 0x100000) {
        return NULL;
    }
    *end_out = mbi + total_size;
    return mbi + 8;
}

/*
 * The tag at p if its header and declared size fit before end (size >= 8),
 * else NULL. A zero size used to spin forever; sizes past the MBI read garbage.
 */
static struct multiboot_tag* mbi_tag_at(uint8_t* p, uint8_t* end) {
    if (p > end || (size_t)(end - p) < sizeof(struct multiboot_tag)) {
        return NULL;
    }
    struct multiboot_tag* tag = (struct multiboot_tag*)p;
    if (tag->type == MULTIBOOT_TAG_TYPE_END) {
        return tag;
    }
    if (tag->size < sizeof(struct multiboot_tag) || tag->size > (size_t)(end - p)) {
        return NULL;
    }
    return tag;
}

static uint8_t* mbi_next_tag(struct multiboot_tag* tag) {
    return (uint8_t*)tag + ((tag->size + 7u) & ~7u);
}

/* Entries in an MMAP tag whose size was already bounds-checked; 0 if malformed. */
static uint32_t mmap_tag_entry_count(const struct multiboot_tag_mmap* mmap_tag) {
    if (mmap_tag->size < sizeof(struct multiboot_tag_mmap) ||
        mmap_tag->entry_size < sizeof(struct multiboot_mmap_entry)) {
        return 0;
    }
    return (uint32_t)((mmap_tag->size - sizeof(struct multiboot_tag_mmap)) / mmap_tag->entry_size);
}

static void multiboot2_walk_tags(bool framebuffer_only) {
    FramebufferInfo tag8 = {0};
    FramebufferInfo tag7 = {0};
    uint8_t* end = NULL;
    uint8_t* p = mbi_first_tag(&end);
    struct multiboot_tag* tag;
    /* fb_info is only replaced when a tag provides one: on UEFI it already
     * holds the GOP handoff and the loader-built MBI has no tag 7/8. */
    if (!p) {
        boot_serial_putc(multiboot2_info_ptr == 0 ? 'M' : '?');
        return;
    }
    while ((tag = mbi_tag_at(p, end)) != NULL && tag->type != MULTIBOOT_TAG_TYPE_END) {
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
                    uint32_t num_entries = mmap_tag_entry_count(mmap_tag);
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
        p = mbi_next_tag(tag);
    }
    if (tag8.present) {
        fb_info = tag8;
    } else if (tag7.present) {
        fb_info = tag7;
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

/* Boot volume identity recorded by the UEFI loader (see uefi_boot.h).
 * Returns 1 and fills serial/sectors when known, else 0 (GRUB, old loader). */
int popcorn_boot_volume(uint32_t* serial, uint64_t* sectors) {
    volatile PopcornUefiBootInfo* u = uefi_handoff_ptr();
    if (u->magic != POPCORN_UEFI_MAGIC || u->boot_vol_valid != POPCORN_UEFI_BOOT_VOL_VALID) {
        return 0;
    }
    *serial = u->boot_vol_serial;
    *sectors = u->boot_vol_sectors;
    return 1;
}

void popcorn_apply_uefi_boot_info(void) {
    volatile PopcornUefiBootInfo* u = uefi_handoff_ptr();
    if (u->magic != POPCORN_UEFI_MAGIC) {
        return;
    }
    /* The loader sets the magic even without GOP; framebuffer_addr == 0 then
     * means "no framebuffer" (BltOnly mode, headless, LocateProtocol failed). */
    if (u->framebuffer_addr == 0 || u->framebuffer_width == 0) {
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

    /* UEFI: uefi_start already pointed multiboot2_info_ptr at the loader-built
     * MBI (handoff mbi_phys), whose tags carry the name, meminfo and mmap. */
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
    uint8_t* end = NULL;
    uint8_t* p = mbi_first_tag(&end);
    struct multiboot_tag* tag;
    if (!p) {
        return;
    }
    while ((tag = mbi_tag_at(p, end)) != NULL && tag->type != MULTIBOOT_TAG_TYPE_END) {
        if (tag->type == MULTIBOOT_TAG_TYPE_MMAP) {
            struct multiboot_tag_mmap* mmap_tag = (struct multiboot_tag_mmap*)tag;
            uint32_t num_entries = mmap_tag_entry_count(mmap_tag);
            for (uint32_t i = 0; i < num_entries; i++) {
                struct multiboot_mmap_entry* entry = (struct multiboot_mmap_entry*)
                    ((uint8_t*)mmap_tag->entries + i * mmap_tag->entry_size);
                fn(entry->addr, entry->len, entry->type, user);
            }
        }
        p = mbi_next_tag(tag);
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
    /* total_memory is the sum of AVAILABLE mmap entries only (reserved MMIO
     * windows are excluded), and already includes conventional RAM below 1 MiB. */
    if (sys_info.valid && sys_info.total_memory > 0) {
        return sys_info.total_memory;
    }
    /* mem_upper counts RAM above 1 MiB only, so add the first MiB back. */
    if (sys_info.mem_upper > 0) {
        return (1024ULL * 1024) + ((uint64_t)sys_info.mem_upper * 1024);
    }
    return 1024ULL * 1024;
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

/* COM1 only (no debugcon): multi-character diagnostics (Rust panics, driver tags
 * such as "net: ...") must not land in the debugcon log, whose single-letter boot
 * tags the smoke tests grep for. */
void com1_putc(char c) {
    for (int i = 0; i < 100000; i++) {
        unsigned char s;
        __asm__ volatile("inb %1, %0" : "=a"(s) : "Nd"((unsigned short)0x3FD));
        if (s & 0x20) {
            break;
        }
    }
    __asm__ volatile("outb %0, %1" : : "a"(c), "Nd"((unsigned short)0x3F8));
}

void com1_puts(const char* s) {
    while (s && *s) {
        if (*s == '\n') {
            com1_putc('\r');
        }
        com1_putc(*s++);
    }
}

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

    /* UEFI path on CPUs with PDPE1GB: low identity uses 1 GiB pages in
     * page_table_l3 (without PDPE1GB uefi_start builds the 2 MiB layout above
     * and these L3 entries are non-leaf, so they are skipped). Mark the
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

