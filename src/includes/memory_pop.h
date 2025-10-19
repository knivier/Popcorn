// src/includes/memory_pop.h
#ifndef MEMORY_POP_H
#define MEMORY_POP_H

#include "pop_module.h"
#include <stdint.h>
#include <stdbool.h>

// Memory region types
#define MEMORY_TYPE_AVAILABLE        1
#define MEMORY_TYPE_RESERVED         2
#define MEMORY_TYPE_ACPI_RECLAIMABLE 3
#define MEMORY_TYPE_ACPI_NVS         4
#define MEMORY_TYPE_BAD              5

// Memory statistics
typedef struct {
    uint64_t total_physical;
    uint64_t total_available;
    uint64_t total_reserved;
    uint64_t total_used;        // Simulated for now
    uint32_t num_regions;
    uint32_t num_available_regions;
} MemoryStats;

// Function declarations
void memory_pop_func(unsigned int start_pos);
void memory_print_map(void);
void memory_print_usage(void);
void memory_print_stats(void);
void memory_calculate_stats(void);
const MemoryStats* memory_get_stats(void);

// Module definition
extern const PopModule memory_module;

#endif // MEMORY_POP_H

