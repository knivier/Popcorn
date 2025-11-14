// src/core/memory.c
#include "../includes/memory.h"
#include "../includes/console.h"
#include "../includes/multiboot2.h"
#include "../includes/utils.h"
#include <stddef.h>
#include <stdint.h>

// External multiboot2 pointer
extern uint64_t multiboot2_info_ptr;

// External functions
extern ConsoleState console_state;
extern void console_draw_separator(unsigned int y, unsigned char color);

// Format memory size in human-readable form
static void format_memory_size(uint64_t bytes, char* buffer, size_t buffer_size) {
    (void)buffer_size;  // Suppress unused parameter warning
    if (bytes >= 1024 * 1024 * 1024) {
        // GB
        uint64_t gb = bytes / (1024 * 1024 * 1024);
        uint64_t mb = (bytes % (1024 * 1024 * 1024)) / (1024 * 1024);
        int_to_str((int)gb, buffer);
        int len = strlen_simple(buffer);
        buffer[len++] = '.';
        int_to_str((int)(mb / 102), buffer + len);
        len += strlen_simple(buffer + len);
        buffer[len++] = 'G';
        buffer[len++] = 'B';
        buffer[len] = '\0';
    } else if (bytes >= 1024 * 1024) {
        // MB
        uint64_t mb = bytes / (1024 * 1024);
        uint64_t kb = (bytes % (1024 * 1024)) / 1024;
        int_to_str((int)mb, buffer);
        int len = strlen_simple(buffer);
        buffer[len++] = '.';
        int_to_str((int)(kb / 102), buffer + len);
        len += strlen_simple(buffer + len);
        buffer[len++] = 'M';
        buffer[len++] = 'B';
        buffer[len] = '\0';
    } else if (bytes >= 1024) {
        // KB
        uint64_t kb = bytes / 1024;
        uint64_t b = bytes % 1024;
        int_to_str((int)kb, buffer);
        int len = strlen_simple(buffer);
        buffer[len++] = '.';
        int_to_str((int)(b / 102), buffer + len);
        len += strlen_simple(buffer + len);
        buffer[len++] = 'K';
        buffer[len++] = 'B';
        buffer[len] = '\0';
    } else {
        // Bytes
        int_to_str((int)bytes, buffer);
        int len = strlen_simple(buffer);
        buffer[len++] = 'B';
        buffer[len] = '\0';
    }
}

// Memory management structures
typedef struct {
    void* base;
    size_t size;
    bool is_free;
    struct mem_block* next;
    struct mem_block* prev;
} mem_block;

typedef struct {
    mem_block* free_list;
    mem_block* allocated_list;
    size_t total_size;
    size_t free_size;
    size_t allocated_size;
    uint32_t total_blocks;
    uint32_t free_blocks;
} memory_pool;

// Global memory pools for different zones
static memory_pool dma_pool = {0};
static memory_pool normal_pool = {0};
static memory_pool highmem_pool = {0};

// Memory statistics
static KernelMemoryStats mem_stats = {0};

// Memory block pool (simplified - in real system would use pages)
#define MAX_MEMORY_BLOCKS 1024
static mem_block memory_blocks[MAX_MEMORY_BLOCKS];
static uint32_t memory_block_index = 0;

// Initialize memory management
void memory_init(void) {
    // Initialize memory pools
    dma_pool.free_list = NULL;
    dma_pool.allocated_list = NULL;
    dma_pool.total_size = 0;
    dma_pool.free_size = 0;
    dma_pool.allocated_size = 0;
    dma_pool.total_blocks = 0;
    dma_pool.free_blocks = 0;
    
    normal_pool.free_list = NULL;
    normal_pool.allocated_list = NULL;
    normal_pool.total_size = 0;
    normal_pool.free_size = 0;
    normal_pool.allocated_size = 0;
    normal_pool.total_blocks = 0;
    normal_pool.free_blocks = 0;
    
    highmem_pool.free_list = NULL;
    highmem_pool.allocated_list = NULL;
    highmem_pool.total_size = 0;
    highmem_pool.free_size = 0;
    highmem_pool.allocated_size = 0;
    highmem_pool.total_blocks = 0;
    highmem_pool.free_blocks = 0;
    
    // Initialize memory statistics
    mem_stats.total_pages = 0;
    mem_stats.free_pages = 0;
    mem_stats.used_pages = 0;
    mem_stats.reserved_pages = 0;
    mem_stats.total_bytes = 0;
    mem_stats.free_bytes = 0;
    mem_stats.used_bytes = 0;
    
    // Get memory information from multiboot2
    if (multiboot2_info_ptr != 0) {
        // Parse memory map and initialize pools
        // This is simplified - in a real system would parse multiboot2 memory map
        mem_stats.total_bytes = 256 * 1024 * 1024;  // Assume 256MB
        mem_stats.free_bytes = mem_stats.total_bytes - (2 * 1024 * 1024);  // Reserve 2MB for kernel
        mem_stats.used_bytes = 2 * 1024 * 1024;
        
        mem_stats.total_pages = mem_stats.total_bytes / PAGE_SIZE;
        mem_stats.free_pages = mem_stats.free_bytes / PAGE_SIZE;
        mem_stats.used_pages = mem_stats.used_bytes / PAGE_SIZE;
        
        // Initialize normal pool with available memory
        normal_pool.total_size = mem_stats.free_bytes;
        normal_pool.free_size = mem_stats.free_bytes;
    }
    
    console_println_color("Memory management initialized", CONSOLE_SUCCESS_COLOR);
}

// Allocate memory
void* kmalloc(size_t size, uint32_t flags) {
    if (size == 0) {
        return NULL;
    }
    
    // Align size to page boundary
    size = align_size(size, PAGE_SIZE);
    
    // Choose appropriate zone based on flags
    MemoryZone zone = ZONE_NORMAL;
    if (flags & MEM_ALLOC_DMA) {
        zone = ZONE_DMA;
    } else if (flags & MEM_ALLOC_HIGHMEM) {
        zone = ZONE_HIGHMEM;
    }
    
    // Allocate from appropriate zone
    void* ptr = zone_alloc(zone, size, flags);
    
    if (ptr && (flags & MEM_ALLOC_ZERO)) {
        memory_zero(ptr, size);
    }
    
    return ptr;
}

// Find the memory block for a given pointer
static mem_block* find_block_for_ptr(void* ptr) {
    if (!ptr) {
        return NULL;
    }
    
    // Search through all allocated blocks to find the one matching this pointer
    for (uint32_t i = 0; i < memory_block_index; i++) {
        mem_block* block = &memory_blocks[i];
        if (block->base == ptr && !block->is_free) {
            return block;
        }
    }
    
    return NULL;
}

// Free memory
void kfree(void* ptr) {
    if (!ptr) {
        return;
    }
    
    // Find the block for this pointer
    mem_block* block = find_block_for_ptr(ptr);
    
    if (!block) {
        // Pointer not found in allocated blocks - invalid pointer!
        // In a real kernel, this would be a security violation
        // For now, silently ignore (could log error in production)
        return;
    }
    
    if (block->is_free) {
        // Double-free detected - security issue
        return;
    }
    
    // Mark as free
    block->is_free = true;
    
    // Update pool statistics (find which pool this belongs to)
    // For simplicity, assume normal pool
    normal_pool.free_size += block->size;
    normal_pool.allocated_size -= block->size;
    normal_pool.free_blocks++;
    normal_pool.total_blocks--;
    
    // Update global statistics
    mem_stats.free_bytes += block->size;
    mem_stats.used_bytes -= block->size;
}

// Check if a pointer is a valid allocation
bool is_valid_allocation(void* ptr) {
    if (!ptr) {
        return false;
    }
    
    mem_block* block = find_block_for_ptr(ptr);
    return (block != NULL && !block->is_free);
}

// Reallocate memory
void* krealloc(void* ptr, size_t size) {
    if (!ptr) {
        return kmalloc(size, MEM_ALLOC_NORMAL);
    }
    
    if (size == 0) {
        kfree(ptr);
        return NULL;
    }
    
    // Simplified realloc - in real system would try to extend in place
    void* new_ptr = kmalloc(size, MEM_ALLOC_NORMAL);
    if (new_ptr) {
        // Copy old data (simplified - would need to track old size)
        memory_copy(new_ptr, ptr, size);
        kfree(ptr);
    }
    
    return new_ptr;
}

// Allocate and zero memory
void* kcalloc(size_t count, size_t size) {
    size_t total_size = count * size;
    return kmalloc(total_size, MEM_ALLOC_ZERO);
}

// Allocate pages
void* alloc_pages(size_t num_pages, uint32_t flags) {
    size_t size = num_pages * PAGE_SIZE;
    return kmalloc(size, flags);
}

// Free pages
void free_pages(void* ptr, size_t num_pages) {
    (void)num_pages;  // Suppress unused parameter warning
    kfree(ptr);
}

// Check if page is allocated
bool is_page_allocated(void* ptr) {
    // Simplified - in real system would check page tables
    return ptr != NULL;
}

// Convert page number to virtual address
void* page_to_virt(uint64_t page) {
    return (void*)(page * PAGE_SIZE);
}

// Convert virtual address to page number
uint64_t virt_to_page(void* ptr) {
    return ((uintptr_t)ptr) >> PAGE_SHIFT;
}

// Get memory statistics
KernelMemoryStats* memory_get_stats(void) {
    return &mem_stats;
}

// Print memory statistics
void kernel_memory_print_stats(void) {
    console_newline();
    console_println_color("=== MEMORY STATISTICS ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    char buffer[64];
    
    // Total memory
    console_print_color("Total Memory: ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.total_bytes, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    // Free memory
    console_print_color("Free Memory: ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.free_bytes, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    
    // Used memory
    console_print_color("Used Memory: ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.used_bytes, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_WARNING_COLOR);
    
    // Pages
    console_print_color("Total Pages: ", CONSOLE_INFO_COLOR);
    int_to_str(mem_stats.total_pages, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_print_color("Free Pages: ", CONSOLE_INFO_COLOR);
    int_to_str(mem_stats.free_pages, buffer);
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

// Allocate from specific zone
void* zone_alloc(MemoryZone zone, size_t size, uint32_t flags) {
    (void)flags;  // Suppress unused parameter warning
    memory_pool* pool;
    
    switch (zone) {
        case ZONE_DMA:
            pool = &dma_pool;
            break;
        case ZONE_NORMAL:
            pool = &normal_pool;
            break;
        case ZONE_HIGHMEM:
            pool = &highmem_pool;
            break;
        default:
            return NULL;
    }
    
    if (pool->free_size < size) {
        return NULL;  // Not enough memory
    }
    
    // Simplified allocation - in real system would use proper allocator
    if (memory_block_index >= MAX_MEMORY_BLOCKS) {
        return NULL;
    }
    
    mem_block* block = &memory_blocks[memory_block_index++];
    block->base = (void*)(uintptr_t)(0x1000000 + memory_block_index * PAGE_SIZE);  // Simplified address
    block->size = size;
    block->is_free = false;
    block->next = NULL;
    block->prev = NULL;
    
    pool->allocated_size += size;
    pool->free_size -= size;
    pool->total_blocks++;
    pool->free_blocks--;
    
    mem_stats.used_bytes += size;
    mem_stats.free_bytes -= size;
    
    return block->base;
}

// Free from specific zone
void zone_free(MemoryZone zone, void* ptr, size_t size) {
    (void)zone;   // Suppress unused parameter warning
    (void)ptr;    // Suppress unused parameter warning
    // Simplified - would need to find and free the block
    mem_stats.free_bytes += size;
    mem_stats.used_bytes -= size;
}

// Align size to boundary
size_t align_size(size_t size, size_t alignment) {
    return (size + alignment - 1) & ~(alignment - 1);
}

// Check if pointer is aligned
bool is_aligned(void* ptr, size_t alignment) {
    return ((uintptr_t)ptr & (alignment - 1)) == 0;
}

// Zero memory
void memory_zero(void* ptr, size_t size) {
    if (!ptr) return;
    
    uint8_t* byte_ptr = (uint8_t*)ptr;
    for (size_t i = 0; i < size; i++) {
        byte_ptr[i] = 0;
    }
}

// Copy memory
void memory_copy(void* dest, const void* src, size_t size) {
    if (!dest || !src) return;
    
    uint8_t* dest_ptr = (uint8_t*)dest;
    const uint8_t* src_ptr = (const uint8_t*)src;
    
    for (size_t i = 0; i < size; i++) {
        dest_ptr[i] = src_ptr[i];
    }
}

// Debug print memory state
void memory_debug_print(void) {
    char buffer[64];
    console_println_color("Memory Debug Info:", CONSOLE_INFO_COLOR);
    console_println_color("Normal Pool:", CONSOLE_INFO_COLOR);
    console_print_color("  Total Size: ", CONSOLE_FG_COLOR);
    int_to_str(normal_pool.total_size, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_print_color("  Free Size: ", CONSOLE_FG_COLOR);
    int_to_str(normal_pool.free_size, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_print_color("  Allocated Size: ", CONSOLE_FG_COLOR);
    int_to_str(normal_pool.allocated_size, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
}

// Check memory integrity
bool memory_check_integrity(void) {
    // Simplified integrity check
    return mem_stats.total_bytes == (mem_stats.free_bytes + mem_stats.used_bytes);
}
