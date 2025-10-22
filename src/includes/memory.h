// src/includes/memory.h
#ifndef MEMORY_H
#define MEMORY_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

// Memory allocation flags
#define MEM_ALLOC_NORMAL    0x00
#define MEM_ALLOC_ZERO      0x01
#define MEM_ALLOC_DMA       0x02
#define MEM_ALLOC_HIGHMEM   0x04

// Memory zones
typedef enum {
    ZONE_DMA,      // Direct Memory Access
    ZONE_NORMAL,   // Normal memory
    ZONE_HIGHMEM   // High memory
} MemoryZone;

// Page size constants
#define PAGE_SIZE 4096
#define PAGE_SHIFT 12
#define PAGE_MASK (~(PAGE_SIZE - 1))

// Memory allocation result
typedef struct {
    void* ptr;
    size_t size;
    bool success;
    uint32_t flags;
} AllocResult;

// Memory statistics
typedef struct {
    uint64_t total_pages;
    uint64_t free_pages;
    uint64_t used_pages;
    uint64_t reserved_pages;
    uint64_t total_bytes;
    uint64_t free_bytes;
    uint64_t used_bytes;
} KernelMemoryStats;

// Memory management functions
void memory_init(void);
void* kmalloc(size_t size, uint32_t flags);
void kfree(void* ptr);
void* krealloc(void* ptr, size_t size);
void* kcalloc(size_t count, size_t size);

// Page management
void* alloc_pages(size_t num_pages, uint32_t flags);
void free_pages(void* ptr, size_t num_pages);
bool is_page_allocated(void* ptr);
void* page_to_virt(uint64_t page);
uint64_t virt_to_page(void* ptr);

// Memory statistics
KernelMemoryStats* memory_get_stats(void);
void kernel_memory_print_stats(void);

// Memory zones
void* zone_alloc(MemoryZone zone, size_t size, uint32_t flags);
void zone_free(MemoryZone zone, void* ptr, size_t size);

// Utility functions
size_t align_size(size_t size, size_t alignment);
bool is_aligned(void* ptr, size_t alignment);
void memory_zero(void* ptr, size_t size);
void memory_copy(void* dest, const void* src, size_t size);

// Debug functions
void memory_debug_print(void);
bool memory_check_integrity(void);

#endif // MEMORY_H
