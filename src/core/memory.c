// src/core/memory.c — bitmap physical allocator, identity-mapped 4K pages
#include "../includes/memory.h"
#include "../includes/console.h"
#include "../includes/multiboot2.h"
#include "../includes/utils.h"
#include <stddef.h>
#include <stdint.h>

extern uint64_t multiboot2_info_ptr;
extern char __kernel_start[];
extern char __kernel_end[];

extern ConsoleState console_state;
extern void console_draw_separator(unsigned int y, unsigned char color);

/* 1GB identity-mapped in kernel.asm (512 x 2MB huge pages) = 2^18 4K frames */
#define PMM_MAX_4K_FRAMES (1U << 18)
#define PMM_BITMAP_BYTES (PMM_MAX_4K_FRAMES / 8U)

/* Bit 1 = used, 0 = free */
static uint8_t pmm_bitmap[PMM_BITMAP_BYTES] __attribute__((aligned(4096)));
static bool pmm_ready = false;

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

#define MAX_MEMORY_BLOCKS 1024
static mem_block memory_blocks[MAX_MEMORY_BLOCKS];
static uint32_t memory_block_index = 0;

static void pmm_set_used(uint32_t f) {
    if (f >= PMM_MAX_4K_FRAMES) {
        return;
    }
    pmm_bitmap[f >> 3U] |= (uint8_t)(1U << (f & 7U));
}

static void pmm_set_free(uint32_t f) {
    if (f >= PMM_MAX_4K_FRAMES) {
        return;
    }
    pmm_bitmap[f >> 3U] &= (uint8_t)~(1U << (f & 7U));
}

static int pmm_frame_free(uint32_t f) {
    if (f >= PMM_MAX_4K_FRAMES) {
        return 0;
    }
    return (pmm_bitmap[f >> 3U] & (1U << (f & 7U))) == 0;
}

static uint32_t pmm_count_free(void) {
    uint32_t n = 0;
    for (uint32_t f = 0; f < PMM_MAX_4K_FRAMES; f++) {
        if (pmm_frame_free(f)) {
            n++;
        }
    }
    return n;
}

static void pmm_mark_range_used(uint64_t pstart, uint64_t pend) {
    if (pend <= pstart) {
        return;
    }
    pstart = pstart & ~(uint64_t)(PAGE_SIZE - 1);
    pend = (pend + PAGE_SIZE - 1) & ~(uint64_t)(PAGE_SIZE - 1);
    for (uint64_t a = pstart; a < pend; a += PAGE_SIZE) {
        uint32_t f = (uint32_t)(a / PAGE_SIZE);
        pmm_set_used(f);
    }
}

static void pmm_give_free_range(uint64_t pstart, uint64_t plen) {
    if (plen == 0) {
        return;
    }
    uint64_t end = pstart + plen;
    pstart = (pstart + PAGE_SIZE - 1) & ~(uint64_t)(PAGE_SIZE - 1);
    for (uint64_t a = pstart; a < end; a += PAGE_SIZE) {
        uint32_t f = (uint32_t)(a / PAGE_SIZE);
        if (f < PMM_MAX_4K_FRAMES) {
            pmm_set_free(f);
        }
    }
}

struct mmap_free_ctx { int did_free; };

static void mmap_unreserve_cb(uint64_t base, uint64_t len, uint32_t type, void* user) {
    struct mmap_free_ctx* ctx = (struct mmap_free_ctx*)user;
    if (type == MULTIBOOT_MEMORY_AVAILABLE) {
        pmm_give_free_range(base, len);
        ctx->did_free = 1;
    }
}

static int32_t pmm_alloc_contig(uint32_t n) {
    if (n == 0 || n > PMM_MAX_4K_FRAMES) {
        return -1;
    }
    for (uint32_t i = 0; i + n <= PMM_MAX_4K_FRAMES; i++) {
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
            return (int32_t)i;
        }
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

    struct mmap_free_ctx ctx = {0};
    multiboot2_foreach_mmap(mmap_unreserve_cb, &ctx);

    if (!ctx.did_free) {
        SystemInfo* inf = multiboot2_get_info();
        if (inf->mem_upper > 0) {
            uint64_t from = 1024U * 1024U;
            uint64_t to = from + (uint64_t)inf->mem_upper * 1024U;
            if (to > (1ULL << 30)) {
                to = 1ULL << 30;
            }
            pmm_give_free_range(from, to - from);
        }
    }

    /* Reserve low 1 MiB: IVT, BDA, etc. (also avoids handing out 0 / NULL frames). */
    pmm_mark_range_used(0, 0x100000u);

    pmm_mark_range_used((uint64_t)(uintptr_t)__kernel_start, (uint64_t)(uintptr_t)__kernel_end);

    if (multiboot2_info_ptr != 0) {
        const uint8_t* m = (const uint8_t*)(uintptr_t)multiboot2_info_ptr;
        uint32_t sz = *(const uint32_t*)m;
        pmm_mark_range_used(multiboot2_info_ptr, (uint64_t)sz);
    }

    pmm_ready = true;
    {
        uint32_t fr = pmm_count_free();
        mem_stats.reserved_pages = 0;
        mem_stats.total_pages = PMM_MAX_4K_FRAMES;
        mem_stats.free_pages = fr;
        mem_stats.used_pages = PMM_MAX_4K_FRAMES - fr;
        mem_stats.total_bytes = (uint64_t)PMM_MAX_4K_FRAMES * PAGE_SIZE;
        mem_stats.free_bytes = (uint64_t)fr * PAGE_SIZE;
        mem_stats.used_bytes = mem_stats.total_bytes - mem_stats.free_bytes;
        normal_pool.total_size = mem_stats.free_bytes;
        normal_pool.free_size = mem_stats.free_bytes;
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
    console_println_color("Physical memory: bitmap pmm, 1 GiB identity-mapped", CONSOLE_SUCCESS_COLOR);
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
    uint32_t f0 = (uint32_t)((uintptr_t)ptr / PAGE_SIZE);
    pmm_free_range_frames(f0, (uint32_t)npg);
    b->is_free = true;
    if (mem_stats.used_bytes >= b->size) {
        mem_stats.used_bytes -= b->size;
    }
    if (mem_stats.total_bytes > mem_stats.used_bytes) {
        mem_stats.free_bytes = mem_stats.total_bytes - mem_stats.used_bytes;
    }
    mem_stats.free_pages = pmm_count_free();
    mem_stats.used_pages = PMM_MAX_4K_FRAMES - mem_stats.free_pages;
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
    uint32_t f = (uint32_t)((uintptr_t)ptr / PAGE_SIZE);
    if (f >= PMM_MAX_4K_FRAMES) {
        return false;
    }
    return (pmm_bitmap[f >> 3U] & (1U << (f & 7U))) != 0;
}

void* page_to_virt(uint64_t page) {
    return (void*)(page * PAGE_SIZE);
}

uint64_t virt_to_page(void* p) {
    return ((uintptr_t)p) >> PAGE_SHIFT;
}

KernelMemoryStats* memory_get_stats(void) {
    if (pmm_ready) {
        mem_stats.free_pages = pmm_count_free();
        mem_stats.used_pages = PMM_MAX_4K_FRAMES - mem_stats.free_pages;
        mem_stats.free_bytes = (uint64_t)mem_stats.free_pages * PAGE_SIZE;
        mem_stats.used_bytes = (uint64_t)mem_stats.used_pages * PAGE_SIZE;
    }
    return &mem_stats;
}

void kernel_memory_print_stats(void) {
    char buffer[64];
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
    if (memory_block_index >= MAX_MEMORY_BLOCKS) {
        pmm_free_range_frames((uint32_t)st, npg);
        return NULL;
    }
    void* base = (void*)(uintptr_t)((uint32_t)st * PAGE_SIZE);
    mem_block* b = &memory_blocks[memory_block_index++];
    b->base = base;
    b->size = (size_t)npg * PAGE_SIZE;
    b->is_free = false;
    mem_stats.used_bytes += b->size;
    mem_stats.free_bytes = mem_stats.total_bytes - mem_stats.used_bytes;
    mem_stats.free_pages = pmm_count_free();
    mem_stats.used_pages = PMM_MAX_4K_FRAMES - mem_stats.free_pages;
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
    console_println_color("PMM bitmap, first 1 GiB identity", CONSOLE_INFO_COLOR);
    int_to_str((int)pmm_count_free(), b);
    console_print_color("Free 4K frames: ", CONSOLE_INFO_COLOR);
    console_println_color(b, CONSOLE_FG_COLOR);
}

bool memory_check_integrity(void) {
    return mem_stats.total_bytes == mem_stats.free_bytes + mem_stats.used_bytes;
}
