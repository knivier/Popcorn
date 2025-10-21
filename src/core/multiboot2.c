// src/core/multiboot2.c
#include "../includes/multiboot2.h"
#include "../includes/console.h"
#include <stddef.h>

// Static storage for parsed system info
static SystemInfo sys_info = {0};

// String copy helper
static void strncpy_safe(char* dest, const char* src, size_t max_len) {
    size_t i;
    for (i = 0; i < max_len - 1 && src[i] != '\0'; i++) {
        dest[i] = src[i];
    }
    dest[i] = '\0';
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

    // Check if we have a valid multiboot info pointer
    if (multiboot2_info_ptr == 0) {
        // Use basic memory info from a fallback method
        sys_info.mem_lower = 640;  // Standard lower memory
        sys_info.mem_upper = 0;
        sys_info.valid = true;
        return;
    }

    // The first 8 bytes contain total size and reserved field
    uint32_t* mbi = (uint32_t*)(uintptr_t)multiboot2_info_ptr;
    uint32_t total_size = mbi[0];
    
    // Sanity check
    if (total_size < 8 || total_size > 0x100000) {
        sys_info.mem_lower = 640;
        sys_info.mem_upper = 0;
        sys_info.valid = true;
        return;
    }

    // Start parsing tags (skip first 8 bytes)
    struct multiboot_tag* tag = (struct multiboot_tag*)((uint8_t*)mbi + 8);
    
    while (tag->type != MULTIBOOT_TAG_TYPE_END) {
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
                
                // Calculate total available memory
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
        }
        
        // Move to next tag (tags are 8-byte aligned)
        uint32_t tag_size = tag->size;
        tag = (struct multiboot_tag*)((uint8_t*)tag + ((tag_size + 7) & ~7));
        
        // Safety check to prevent infinite loop
        if ((uintptr_t)tag >= (uintptr_t)mbi + total_size) {
            break;
        }
    }
    
    sys_info.valid = true;
}

/*
 * Get pointer to parsed system info
 */
SystemInfo* multiboot2_get_info(void) {
    return &sys_info;
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

