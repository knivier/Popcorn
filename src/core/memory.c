// src/core/memory.c — bitmap PMM over the direct-map / identity window
#include "../includes/memory.h"
#include "../includes/physmap.h"
#include "../includes/vmm.h"
#include "../includes/console.h"
#include "../includes/multiboot2.h"
#include "../includes/uefi_boot.h"
#include "../includes/utils.h"
#include <stddef.h>
#include <stdint.h>

extern uint64_t multiboot2_info_ptr;
extern char __kernel_start[];
extern char __kernel_end[];
/* Absolute (LMA) range of the multiboot-loaded image from link.ld. */
extern char __kernel_lma_start[];
extern char __kernel_lma_end[];

extern ConsoleState console_state;
extern void console_draw_separator(unsigned int y, unsigned char color);

#define PMM_BITMAP_BYTES ((size_t)(PMM_MAX_4K_FRAMES / 8ull))

/* Bit 1 = used, 0 = free. Covers PMM_TRACK_GIB (see physmap.h). */
static uint8_t pmm_bitmap[PMM_BITMAP_BYTES] __attribute__((aligned(4096)));
static bool pmm_ready = false;
static uint64_t pmm_free_frames = 0;
static uint64_t pmm_ram_bytes = 0; /* sum of AVAILABLE regions accepted into the bitmap */
static uint32_t pmm_scan_hint = 0;

typedef struct {
    void* base;
    size_t size;
    bool is_free;
} mem_block;

typedef struct {
    size_t total_size;
    size_t free_size;
    size_t allocated_size;
} memory_pool;

static memory_pool normal_pool;
static KernelMemoryStats mem_stats;

#define MAX_MEMORY_BLOCKS 2048
static mem_block memory_blocks[MAX_MEMORY_BLOCKS];
static uint32_t memory_block_index = 0;

static void pmm_set_used(uint32_t f) {
    if ((uint64_t)f >= PMM_MAX_4K_FRAMES) {
        return;
    }
    uint8_t bit = (uint8_t)(1U << (f & 7U));
    uint8_t* cell = &pmm_bitmap[f >> 3U];
    if ((*cell & bit) == 0) {
        *cell |= bit;
        if (pmm_free_frames > 0) {
            pmm_free_frames--;
        }
    }
}

static void pmm_set_free(uint32_t f) {
    if ((uint64_t)f >= PMM_MAX_4K_FRAMES) {
        return;
    }
    uint8_t bit = (uint8_t)(1U << (f & 7U));
    uint8_t* cell = &pmm_bitmap[f >> 3U];
    if ((*cell & bit) != 0) {
        *cell &= (uint8_t)~bit;
        pmm_free_frames++;
    }
}

static int pmm_frame_free(uint32_t f) {
    if ((uint64_t)f >= PMM_MAX_4K_FRAMES) {
        return 0;
    }
    return (pmm_bitmap[f >> 3U] & (1U << (f & 7U))) == 0;
}

static uint64_t pmm_count_free(void) {
    return pmm_free_frames;
}

static void pmm_mark_range_used(uint64_t pstart, uint64_t plen) {
    if (plen == 0) {
        return;
    }
    uint64_t pend = pstart + plen;
    pstart = pstart & ~(uint64_t)(PAGE_SIZE - 1);
    pend = (pend + PAGE_SIZE - 1) & ~(uint64_t)(PAGE_SIZE - 1);
    if (pstart >= PMM_TRACK_BYTES) {
        return;
    }
    if (pend > PMM_TRACK_BYTES) {
        pend = PMM_TRACK_BYTES;
    }
    for (uint64_t a = pstart; a < pend; a += PAGE_SIZE) {
        pmm_set_used((uint32_t)(a / PAGE_SIZE));
    }
}

static void pmm_give_free_range(uint64_t pstart, uint64_t plen) {
    if (plen == 0) {
        return;
    }
    uint64_t end = pstart + plen;
    pstart = (pstart + PAGE_SIZE - 1) & ~(uint64_t)(PAGE_SIZE - 1);
    if (pstart >= PMM_TRACK_BYTES) {
        return;
    }
    if (end > PMM_TRACK_BYTES) {
        end = PMM_TRACK_BYTES;
    }
    if (end <= pstart) {
        return;
    }
    pmm_ram_bytes += (end - pstart);
    for (uint64_t a = pstart; a < end; a += PAGE_SIZE) {
        pmm_set_free((uint32_t)(a / PAGE_SIZE));
    }
}

struct mmap_free_ctx { int did_free; };

/*
 * ThinkPad / firmware-keyboard UEFI: Boot Services stay up. EFI "conventional"
 * regions in the memory map are NOT safe to use until ExitBootServices — using
 * them for PMM corrupts firmware and the next conin call reboots the machine.
 */
#define UEFI_FW_SAFE_RAM_BASE (64ULL * 1024ULL * 1024ULL)
#define UEFI_FW_SAFE_RAM_BYTES ((512ULL * 1024ULL * 1024ULL) - UEFI_FW_SAFE_RAM_BASE)

static bool physmem_uefi_firmware_kbd_active(void) {
    if (!multiboot2_is_uefi_boot()) {
        return false;
    }
    volatile PopcornUefiBootInfo* u =
        (volatile PopcornUefiBootInfo*)(uintptr_t)POPCORN_UEFI_HANDOFF_PHYS;
    return u->magic == POPCORN_UEFI_MAGIC &&
           (u->flags & POPCORN_UEFI_FLAG_FIRMWARE_KBD) != 0;
}

static void physmem_reserve_uefi_fixed(void) {
    if (!multiboot2_is_uefi_boot()) {
        return;
    }
    pmm_mark_range_used(POPCORN_UEFI_HANDOFF_PHYS & ~(uint64_t)(PAGE_SIZE - 1),
                        PAGE_SIZE);
    const FramebufferInfo* fb = multiboot2_get_framebuffer();
    if (fb && fb->present && fb->addr != 0) {
        uint64_t span = (uint64_t)fb->pitch * (uint64_t)fb->height;
        if (span == 0) {
            span = (uint64_t)fb->width * (uint64_t)fb->height *
                   (uint64_t)((fb->bpp + 7) / 8);
        }
        if (span > 0) {
            pmm_mark_range_used(fb->addr, span);
        }
    }
}

static void mmap_unreserve_cb(uint64_t base, uint64_t len, uint32_t type, void* user) {
    struct mmap_free_ctx* ctx = (struct mmap_free_ctx*)user;
    if (type != MULTIBOOT_MEMORY_AVAILABLE || len == 0) {
        return;
    }
    if (physmem_uefi_firmware_kbd_active()) {
        return;
    }
    pmm_give_free_range(base, len);
    ctx->did_free = 1;
}

static int32_t pmm_alloc_contig(uint32_t n) {
    if (n == 0 || (uint64_t)n > PMM_MAX_4K_FRAMES) {
        return -1;
    }
    const uint32_t limit = (uint32_t)PMM_MAX_4K_FRAMES;
    uint32_t start = pmm_scan_hint;
    for (uint32_t pass = 0; pass < 2; pass++) {
        for (uint32_t i = start; i + n <= limit; ) {
            uint32_t j;
            for (j = 0; j < n; j++) {
                if (!pmm_frame_free(i + j)) {
                    break;
                }
            }
            if (j == n) {
                for (j = 0; j < n; j++) {
                    pmm_set_used(i + j);
                }
                pmm_scan_hint = i + n;
                return (int32_t)i;
            }
            i += (j == 0) ? 1u : j;
        }
        start = 0; /* wrap once */
    }
    return -1;
}

static void pmm_free_range_frames(uint32_t start_f, uint32_t n) {
    for (uint32_t j = 0; j < n; j++) {
        pmm_set_free(start_f + j);
    }
}

void physmem_init(void) {
    memset(pmm_bitmap, 0xFF, sizeof(pmm_bitmap));
    pmm_free_frames = 0;
    pmm_ram_bytes = 0;
    pmm_scan_hint = 0;

    struct mmap_free_ctx ctx = {0};
    if (!physmem_uefi_firmware_kbd_active()) {
        multiboot2_foreach_mmap(mmap_unreserve_cb, &ctx);
    }

    if (!ctx.did_free) {
        if (physmem_uefi_firmware_kbd_active()) {
            pmm_give_free_range(UEFI_FW_SAFE_RAM_BASE, UEFI_FW_SAFE_RAM_BYTES);
            ctx.did_free = 1;
        } else {
            SystemInfo* inf = multiboot2_get_info();
            if (inf->mem_upper > 0) {
                uint64_t from = 1024U * 1024U;
                uint64_t to = from + (uint64_t)inf->mem_upper * 1024U;
                if (to > PMM_TRACK_BYTES) {
                    to = PMM_TRACK_BYTES;
                }
                pmm_give_free_range(from, to - from);
            }
        }
    }

    /* Reserve low 1 MiB: IVT, BDA, etc. (also avoids handing out 0 / NULL frames). */
    pmm_mark_range_used(0, 0x100000u);
    physmem_reserve_uefi_fixed();

    /* LMA: physical span [__kernel_lma_start, __kernel_lma_end). */
    pmm_mark_range_used(
        (uint64_t)(uintptr_t)__kernel_lma_start,
        (uint64_t)(uintptr_t)__kernel_lma_end - (uint64_t)(uintptr_t)__kernel_lma_start
    );

    if (multiboot2_info_ptr != 0) {
        const uint8_t* m = (const uint8_t*)(uintptr_t)multiboot2_info_ptr;
        uint32_t sz = *(const uint32_t*)m;
        pmm_mark_range_used(multiboot2_info_ptr, (uint64_t)sz);
    }

    pmm_ready = true;
    {
        uint64_t fr = pmm_count_free();
        uint64_t total = pmm_ram_bytes ? pmm_ram_bytes : (fr * PAGE_SIZE);
        mem_stats.reserved_pages = 0;
        mem_stats.total_pages = total / PAGE_SIZE;
        mem_stats.free_pages = fr;
        mem_stats.used_pages = (total / PAGE_SIZE) > fr ? (total / PAGE_SIZE) - fr : 0;
        mem_stats.total_bytes = total;
        mem_stats.free_bytes = fr * PAGE_SIZE;
        mem_stats.used_bytes = mem_stats.total_bytes > mem_stats.free_bytes
                                   ? mem_stats.total_bytes - mem_stats.free_bytes
                                   : 0;
        normal_pool.total_size = (size_t)mem_stats.free_bytes;
        normal_pool.free_size = (size_t)mem_stats.free_bytes;
        normal_pool.allocated_size = 0;
    }
}

static void format_memory_size(uint64_t bytes, char* buffer, size_t bufsz) {
    (void)bufsz;
    if (bytes >= 1024 * 1024 * 1024) {
        uint64_t g = bytes / (1024 * 1024 * 1024);
        int_to_str((int)g, buffer);
    } else if (bytes >= 1024 * 1024) {
        uint64_t m = bytes / (1024 * 1024);
        int_to_str((int)m, buffer);
    } else if (bytes >= 1024) {
        uint64_t k = bytes / 1024;
        int_to_str((int)k, buffer);
    } else {
        int_to_str((int)bytes, buffer);
    }
}

void memory_init(void) {
    memory_block_index = 0;
    normal_pool = (memory_pool){0};
    physmem_init();
    vmm_init();
    console_println_color("Physical memory: bitmap PMM (tracks up to 16 GiB DRAM)", CONSOLE_SUCCESS_COLOR);
    console_println_color("Virtual: direct map 64 GiB @ 0xFFFF800000000000; process PML4 clones upper half", CONSOLE_INFO_COLOR);
}

void* kmalloc(size_t size, uint32_t flags) {
    if (size == 0) {
        return NULL;
    }
    size = align_size(size, PAGE_SIZE);
    MemoryZone z = ZONE_NORMAL;
    if (flags & MEM_ALLOC_DMA) {
        z = ZONE_DMA;
    } else if (flags & MEM_ALLOC_HIGHMEM) {
        z = ZONE_HIGHMEM;
    }
    void* p = zone_alloc(z, size, flags);
    if (p && (flags & MEM_ALLOC_ZERO)) {
        memory_zero(p, size);
    }
    return p;
}

static mem_block* find_block(void* ptr) {
    if (!ptr) {
        return NULL;
    }
    for (uint32_t i = 0; i < memory_block_index; i++) {
        if (memory_blocks[i].base == ptr && !memory_blocks[i].is_free) {
            return &memory_blocks[i];
        }
    }
    return NULL;
}

void kfree(void* ptr) {
    if (!ptr || !pmm_ready) {
        return;
    }
    mem_block* b = find_block(ptr);
    if (!b || b->is_free) {
        return;
    }
    size_t npg = (b->size + PAGE_SIZE - 1) / PAGE_SIZE;
    uint32_t f0 = (uint32_t)(virt_to_phys_direct(ptr) / PAGE_SIZE);
    pmm_free_range_frames(f0, (uint32_t)npg);
    b->is_free = true;
    if (mem_stats.used_bytes >= b->size) {
        mem_stats.used_bytes -= b->size;
    }
    if (mem_stats.total_bytes > mem_stats.used_bytes) {
        mem_stats.free_bytes = mem_stats.total_bytes - mem_stats.used_bytes;
    }
    mem_stats.free_pages = pmm_count_free();
    if (mem_stats.total_pages >= mem_stats.free_pages) {
        mem_stats.used_pages = mem_stats.total_pages - mem_stats.free_pages;
    }
}

bool is_valid_allocation(void* ptr) {
    mem_block* b = find_block(ptr);
    return b != NULL && !b->is_free;
}

void* krealloc(void* ptr, size_t size) {
    if (!ptr) {
        return kmalloc(size, MEM_ALLOC_NORMAL);
    }
    if (size == 0) {
        kfree(ptr);
        return NULL;
    }
    void* n = kmalloc(size, MEM_ALLOC_NORMAL);
    if (n) {
        memory_copy(n, ptr, size);
        kfree(ptr);
    }
    return n;
}

void* kcalloc(size_t c, size_t s) {
    return kmalloc(c * s, MEM_ALLOC_ZERO);
}

void* alloc_pages(size_t num_pages, uint32_t f) {
    if (num_pages == 0) {
        return NULL;
    }
    return kmalloc(num_pages * PAGE_SIZE, f);
}

void free_pages(void* p, size_t n) {
    (void)n;
    kfree(p);
}

bool is_page_allocated(void* ptr) {
    if (!ptr || !pmm_ready) {
        return false;
    }
    uint64_t phys = virt_to_phys_direct(ptr);
    uint32_t f = (uint32_t)(phys / PAGE_SIZE);
    if ((uint64_t)f >= PMM_MAX_4K_FRAMES) {
        return false;
    }
    return (pmm_bitmap[f >> 3U] & (1U << (f & 7U))) != 0;
}

void* page_to_virt(uint64_t page) {
    return phys_to_virt(page * PAGE_SIZE);
}

uint64_t virt_to_page(void* p) {
    return virt_to_phys_direct(p) >> PAGE_SHIFT;
}

KernelMemoryStats* memory_get_stats(void) {
    if (pmm_ready) {
        mem_stats.free_pages = pmm_count_free();
        if (mem_stats.total_pages >= mem_stats.free_pages) {
            mem_stats.used_pages = mem_stats.total_pages - mem_stats.free_pages;
        }
        mem_stats.free_bytes = mem_stats.free_pages * PAGE_SIZE;
        mem_stats.used_bytes = mem_stats.used_pages * PAGE_SIZE;
        mem_stats.total_bytes = mem_stats.total_pages * PAGE_SIZE;
    }
    return &mem_stats;
}

void kernel_memory_print_stats(void) {
    char buffer[64];
    KernelMemoryStats* live = memory_get_stats();
    (void)live;
    console_newline();
    console_println_color("=== MEMORY STATISTICS ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    format_memory_size(mem_stats.total_bytes, buffer, sizeof buffer);
    console_print_color("Total: ", CONSOLE_INFO_COLOR);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    format_memory_size(mem_stats.free_bytes, buffer, sizeof buffer);
    console_print_color("Free:  ", CONSOLE_INFO_COLOR);
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    int_to_str((int)mem_stats.free_pages, buffer);
    console_print_color("Free 4K pages: ", CONSOLE_INFO_COLOR);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

void* zone_alloc(MemoryZone zone, size_t size, uint32_t flags) {
    (void)flags;
    (void)zone;
    if (!pmm_ready || size == 0) {
        return NULL;
    }
    uint32_t npg = (uint32_t)((size + PAGE_SIZE - 1) / PAGE_SIZE);
    if (npg == 0) {
        return NULL;
    }
    int32_t st = pmm_alloc_contig(npg);
    if (st < 0) {
        return NULL;
    }
    /* Reuse a freed slot first: kfree() only marks is_free, so without this the
     * table (and every Rust Vec/Box/FAT op) dies after MAX_MEMORY_BLOCKS total allocs. */
    mem_block* b = NULL;
    for (uint32_t i = 0; i < memory_block_index; i++) {
        if (memory_blocks[i].is_free) {
            b = &memory_blocks[i];
            break;
        }
    }
    if (!b) {
        if (memory_block_index >= MAX_MEMORY_BLOCKS) {
            pmm_free_range_frames((uint32_t)st, npg);
            return NULL;
        }
        b = &memory_blocks[memory_block_index++];
    }
    void* base = (void*)(uintptr_t)((uint32_t)st * PAGE_SIZE);
    b->base = base;
    b->size = (size_t)npg * PAGE_SIZE;
    b->is_free = false;
    mem_stats.used_bytes += b->size;
    mem_stats.free_bytes = mem_stats.total_bytes > mem_stats.used_bytes
                               ? mem_stats.total_bytes - mem_stats.used_bytes
                               : 0;
    mem_stats.free_pages = pmm_count_free();
    if (mem_stats.total_pages >= mem_stats.free_pages) {
        mem_stats.used_pages = mem_stats.total_pages - mem_stats.free_pages;
    }
    return base;
}

void zone_free(MemoryZone z, void* p, size_t s) {
    (void)z;
    (void)s;
    kfree(p);
}

size_t align_size(size_t s, size_t a) {
    return (s + a - 1) & ~(a - 1);
}

bool is_aligned(void* p, size_t a) {
    return ((uintptr_t)p & (a - 1)) == 0;
}

void memory_zero(void* p, size_t n) {
    if (!p) {
        return;
    }
    uint8_t* b = (uint8_t*)p;
    for (size_t i = 0; i < n; i++) {
        b[i] = 0;
    }
}

void memory_copy(void* d, const void* s, size_t n) {
    if (!d || !s) {
        return;
    }
    uint8_t* a = (uint8_t*)d;
    const uint8_t* c = (const uint8_t*)s;
    for (size_t i = 0; i < n; i++) {
        a[i] = c[i];
    }
}

void memory_debug_print(void) {
    char b[32];
    console_println_color("PMM bitmap (16 GiB track / 64 GiB direct map)", CONSOLE_INFO_COLOR);
    int_to_str((int)pmm_count_free(), b);
    console_print_color("Free 4K frames: ", CONSOLE_INFO_COLOR);
    console_println_color(b, CONSOLE_FG_COLOR);
}

bool memory_check_integrity(void) {
    return mem_stats.total_bytes == mem_stats.free_bytes + mem_stats.used_bytes;
}
