// src/pops/memory_pop.c
#include "../includes/memory_pop.h"
#include "../includes/console.h"
#include "../includes/multiboot2.h"
#include <stddef.h>
#include <stdbool.h>

// Static storage for memory stats
static MemoryStats mem_stats = {0};
static bool stats_initialized = false;

// Access console state
extern ConsoleState console_state;

// External multiboot2 pointer
extern uint64_t multiboot2_info_ptr;

// External function to convert int to string
extern void int_to_str(int num, char *str);

// Memory type to string
static const char* memory_type_to_string(uint32_t type) {
    switch (type) {
        case MEMORY_TYPE_AVAILABLE:
            return "Available";
        case MEMORY_TYPE_RESERVED:
            return "Reserved";
        case MEMORY_TYPE_ACPI_RECLAIMABLE:
            return "ACPI Reclaimable";
        case MEMORY_TYPE_ACPI_NVS:
            return "ACPI NVS";
        case MEMORY_TYPE_BAD:
            return "Bad RAM";
        default:
            return "Unknown";
    }
}

// Format memory size in human-readable form
static void format_memory_size(uint64_t bytes, char* buffer, size_t buffer_size) {
    if (bytes >= 1024ULL * 1024 * 1024) {
        uint64_t gb = bytes / (1024ULL * 1024 * 1024);
        int_to_str((int)gb, buffer);
        size_t len = 0;
        while (buffer[len] != '\0' && len < buffer_size - 4) len++;
        if (len < buffer_size - 4) {
            buffer[len++] = ' ';
            buffer[len++] = 'G';
            buffer[len++] = 'B';
            buffer[len] = '\0';
        }
    } else if (bytes >= 1024ULL * 1024) {
        uint64_t mb = bytes / (1024ULL * 1024);
        int_to_str((int)mb, buffer);
        size_t len = 0;
        while (buffer[len] != '\0' && len < buffer_size - 4) len++;
        if (len < buffer_size - 4) {
            buffer[len++] = ' ';
            buffer[len++] = 'M';
            buffer[len++] = 'B';
            buffer[len] = '\0';
        }
    } else if (bytes >= 1024ULL) {
        uint64_t kb = bytes / 1024ULL;
        int_to_str((int)kb, buffer);
        size_t len = 0;
        while (buffer[len] != '\0' && len < buffer_size - 4) len++;
        if (len < buffer_size - 4) {
            buffer[len++] = ' ';
            buffer[len++] = 'K';
            buffer[len++] = 'B';
            buffer[len] = '\0';
        }
    } else {
        int_to_str((int)bytes, buffer);
        size_t len = 0;
        while (buffer[len] != '\0' && len < buffer_size - 3) len++;
        if (len < buffer_size - 3) {
            buffer[len++] = ' ';
            buffer[len++] = 'B';
            buffer[len] = '\0';
        }
    }
}

// Format address as hex string
static void format_address(uint64_t addr, char* buffer) {
    const char hex_chars[] = "0123456789ABCDEF";
    buffer[0] = '0';
    buffer[1] = 'x';
    
    for (int i = 15; i >= 0; i--) {
        buffer[2 + (15 - i)] = hex_chars[(addr >> (i * 4)) & 0xF];
    }
    buffer[18] = '\0';
}

// Calculate memory statistics from multiboot2 memory map
void memory_calculate_stats(void) {
    if (stats_initialized) {
        return;
    }
    
    mem_stats.total_physical = 0;
    mem_stats.total_available = 0;
    mem_stats.total_reserved = 0;
    mem_stats.total_used = 0;
    mem_stats.num_regions = 0;
    mem_stats.num_available_regions = 0;
    
    if (multiboot2_info_ptr == 0) {
        stats_initialized = true;
        return;
    }
    
    uint32_t* mbi = (uint32_t*)(uintptr_t)multiboot2_info_ptr;
    uint32_t total_size = mbi[0];
    
    if (total_size < 8 || total_size > 0x100000) {
        stats_initialized = true;
        return;
    }
    
    struct multiboot_tag {
        uint32_t type;
        uint32_t size;
    };
    
    struct multiboot_mmap_entry {
        uint64_t addr;
        uint64_t len;
        uint32_t type;
        uint32_t zero;
    } __attribute__((packed));
    
    struct multiboot_tag_mmap {
        uint32_t type;
        uint32_t size;
        uint32_t entry_size;
        uint32_t entry_version;
        struct multiboot_mmap_entry entries[0];
    };
    
    struct multiboot_tag* tag = (struct multiboot_tag*)((uint8_t*)mbi + 8);
    
    while (tag->type != 0) {
        if (tag->type == 6) {  // MMAP tag
            struct multiboot_tag_mmap* mmap_tag = (struct multiboot_tag_mmap*)tag;
            uint32_t num_entries = (mmap_tag->size - sizeof(struct multiboot_tag_mmap)) / mmap_tag->entry_size;
            
            for (uint32_t i = 0; i < num_entries; i++) {
                struct multiboot_mmap_entry* entry = (struct multiboot_mmap_entry*)
                    ((uint8_t*)mmap_tag->entries + i * mmap_tag->entry_size);
                
                mem_stats.num_regions++;
                mem_stats.total_physical += entry->len;
                
                if (entry->type == MEMORY_TYPE_AVAILABLE) {
                    mem_stats.total_available += entry->len;
                    mem_stats.num_available_regions++;
                } else {
                    mem_stats.total_reserved += entry->len;
                }
            }
            break;
        }
        
        uint32_t tag_size = tag->size;
        tag = (struct multiboot_tag*)((uint8_t*)tag + ((tag_size + 7) & ~7));
        
        if ((uintptr_t)tag >= (uintptr_t)mbi + total_size) {
            break;
        }
    }
    
    // Simulate used memory (kernel + modules = ~2MB)
    mem_stats.total_used = 2 * 1024 * 1024;
    
    stats_initialized = true;
}

// Print extended memory map
void memory_print_map(void) {
    char buffer[128];
    
    console_newline();
    console_println_color("=== MEMORY MAP ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    if (multiboot2_info_ptr == 0) {
        console_println_color("No memory map available", CONSOLE_ERROR_COLOR);
        return;
    }
    
    uint32_t* mbi = (uint32_t*)(uintptr_t)multiboot2_info_ptr;
    uint32_t total_size = mbi[0];
    
    struct multiboot_tag {
        uint32_t type;
        uint32_t size;
    };
    
    struct multiboot_mmap_entry {
        uint64_t addr;
        uint64_t len;
        uint32_t type;
        uint32_t zero;
    } __attribute__((packed));
    
    struct multiboot_tag_mmap {
        uint32_t type;
        uint32_t size;
        uint32_t entry_size;
        uint32_t entry_version;
        struct multiboot_mmap_entry entries[0];
    };
    
    struct multiboot_tag* tag = (struct multiboot_tag*)((uint8_t*)mbi + 8);
    
    console_println_color("Base Address      | Length           | Type", CONSOLE_INFO_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    int entry_count = 0;
    while (tag->type != 0) {
        if (tag->type == 6) {  // MMAP tag
            struct multiboot_tag_mmap* mmap_tag = (struct multiboot_tag_mmap*)tag;
            uint32_t num_entries = (mmap_tag->size - sizeof(struct multiboot_tag_mmap)) / mmap_tag->entry_size;
            
            for (uint32_t i = 0; i < num_entries && entry_count < 12; i++, entry_count++) {
                struct multiboot_mmap_entry* entry = (struct multiboot_mmap_entry*)
                    ((uint8_t*)mmap_tag->entries + i * mmap_tag->entry_size);
                
                format_address(entry->addr, buffer);
                console_print_color(buffer, CONSOLE_FG_COLOR);
                console_print(" | ");
                
                format_memory_size(entry->len, buffer, sizeof(buffer));
                // Pad to 16 chars
                int len = 0;
                while (buffer[len] != '\0') len++;
                console_print_color(buffer, CONSOLE_FG_COLOR);
                for (int j = len; j < 16; j++) console_print(" ");
                console_print(" | ");
                
                unsigned char color = (entry->type == MEMORY_TYPE_AVAILABLE) ? 
                    CONSOLE_SUCCESS_COLOR : CONSOLE_WARNING_COLOR;
                console_println_color(memory_type_to_string(entry->type), color);
            }
            break;
        }
        
        uint32_t tag_size = tag->size;
        tag = (struct multiboot_tag*)((uint8_t*)tag + ((tag_size + 7) & ~7));
        
        if ((uintptr_t)tag >= (uintptr_t)mbi + total_size) {
            break;
        }
    }
    
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

// Print memory usage
void memory_print_usage(void) {
    char buffer[64];
    
    if (!stats_initialized) {
        memory_calculate_stats();
    }
    
    console_newline();
    console_println_color("=== MEMORY USAGE ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    console_print_color("Total Available: ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.total_available, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    
    console_print_color("Total Used:      ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.total_used, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_WARNING_COLOR);
    
    uint64_t free = mem_stats.total_available - mem_stats.total_used;
    console_print_color("Total Free:      ", CONSOLE_INFO_COLOR);
    format_memory_size(free, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    
    // Calculate percentage
    if (mem_stats.total_available > 0) {
        uint32_t percent = (uint32_t)((mem_stats.total_used * 100) / mem_stats.total_available);
        console_print_color("Usage:           ", CONSOLE_INFO_COLOR);
        int_to_str(percent, buffer);
        console_print_color(buffer, CONSOLE_FG_COLOR);
        console_println("%");
    }
    
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

// Print memory statistics
void memory_print_stats(void) {
    char buffer[64];
    
    if (!stats_initialized) {
        memory_calculate_stats();
    }
    
    console_newline();
    console_println_color("=== MEMORY STATISTICS ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    console_print_color("Total Physical:  ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.total_physical, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_print_color("Total Available: ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.total_available, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    
    console_print_color("Total Reserved:  ", CONSOLE_INFO_COLOR);
    format_memory_size(mem_stats.total_reserved, buffer, sizeof(buffer));
    console_println_color(buffer, CONSOLE_WARNING_COLOR);
    
    console_print_color("Total Regions:   ", CONSOLE_INFO_COLOR);
    int_to_str(mem_stats.num_regions, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_print_color("Avail Regions:   ", CONSOLE_INFO_COLOR);
    int_to_str(mem_stats.num_available_regions, buffer);
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

// Get memory stats pointer
const MemoryStats* memory_pop_get_stats(void) {
    if (!stats_initialized) {
        memory_calculate_stats();
    }
    return &mem_stats;
}

// Pop function
void memory_pop_func(unsigned int start_pos) {
    (void)start_pos;
    
    // Initialize stats on first call
    if (!stats_initialized) {
        memory_calculate_stats();
    }
}

// Pop module definition
const PopModule memory_module = {
    .name = "memory",
    .message = "Memory management and statistics",
    .pop_function = memory_pop_func
};

