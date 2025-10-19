// src/sysinfo_pop.c
#include "includes/sysinfo_pop.h"
#include "includes/console.h"
#include "includes/multiboot2.h"
#include <stdbool.h>
#include <stddef.h>

// Static storage for CPU info
static CPUInfo cpu_info = {0};
static bool cpu_info_initialized = false;

// Access console state to save/restore cursor
extern ConsoleState console_state;

// External function to convert int to string
extern void int_to_str(int num, char *str);

// External keyboard functions
extern char read_port(unsigned short port);

// Keyboard ports
#define KEYBOARD_DATA_PORT 0x60
#define KEYBOARD_STATUS_PORT 0x64

/*
 * Detect CPU information using CPUID
 */
void sysinfo_detect_cpu(void) {
    if (cpu_info_initialized) {
        return;
    }

    // Get CPU vendor string
    cpuid_get_vendor(cpu_info.vendor);
    cpu_info.vendor[12] = '\0';  // Ensure null termination

    // Get CPU features and version info
    uint32_t cpuid_data[4];
    cpuid_get_features(cpuid_data);

    // Parse processor version information (EAX)
    uint32_t eax = cpuid_data[0];
    cpu_info.stepping = eax & 0xF;
    cpu_info.model = (eax >> 4) & 0xF;
    cpu_info.family = (eax >> 8) & 0xF;
    
    // Extended model and family
    if (cpu_info.family == 0xF) {
        cpu_info.family += (eax >> 20) & 0xFF;
    }
    if (cpu_info.family == 0x6 || cpu_info.family == 0xF) {
        cpu_info.model += ((eax >> 16) & 0xF) << 4;
    }

    // Parse feature flags
    uint32_t edx = cpuid_data[3];  // Feature flags (EDX)
    uint32_t ecx = cpuid_data[2];  // Feature flags (ECX)

    cpu_info.has_fpu = (edx & (1 << 0)) != 0;   // FPU
    cpu_info.has_apic = (edx & (1 << 9)) != 0;  // APIC
    cpu_info.has_sse = (edx & (1 << 25)) != 0;  // SSE
    cpu_info.has_sse2 = (edx & (1 << 26)) != 0; // SSE2
    cpu_info.has_sse3 = (ecx & (1 << 0)) != 0;  // SSE3
    cpu_info.has_avx = (ecx & (1 << 28)) != 0;  // AVX

    cpu_info_initialized = true;
}

/*
 * Get pointer to CPU info
 */
const CPUInfo* sysinfo_get_cpu_info(void) {
    if (!cpu_info_initialized) {
        sysinfo_detect_cpu();
    }
    return &cpu_info;
}

/*
 * Format memory size in human-readable form
 */
static void format_memory_size(uint64_t bytes, char* buffer, size_t buffer_size) {
    if (bytes >= 1024ULL * 1024 * 1024) {
        // GB
        uint64_t gb = bytes / (1024ULL * 1024 * 1024);
        int_to_str((int)gb, buffer);
        
        // Append " GB"
        size_t len = 0;
        while (buffer[len] != '\0' && len < buffer_size - 4) len++;
        if (len < buffer_size - 4) {
            buffer[len++] = ' ';
            buffer[len++] = 'G';
            buffer[len++] = 'B';
            buffer[len] = '\0';
        }
    } else if (bytes >= 1024ULL * 1024) {
        // MB
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
        // KB
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
        // Bytes
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

/*
 * Print full system information (called by 'sysinfo' command)
 * Compact version to fit on one screen (25 lines)
 */
void sysinfo_print_full(void) {
    char buffer[128];
    
    // Detect CPU if not already done
    if (!cpu_info_initialized) {
        sysinfo_detect_cpu();
    }

    console_newline();
    console_println_color("=== SYSTEM INFORMATION ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);

    // Kernel & Architecture on one line
    console_print_color("Kernel: ", CONSOLE_INFO_COLOR);
    console_print_color("Popcorn v0.5", CONSOLE_SUCCESS_COLOR);
    console_print_color("  Architecture: ", CONSOLE_INFO_COLOR);
    console_println_color("x86_64 (64-bit long mode)", CONSOLE_FG_COLOR);

    // Bootloader
    console_print_color("Bootloader: ", CONSOLE_INFO_COLOR);
    console_println_color(multiboot2_get_bootloader_name(), CONSOLE_FG_COLOR);

    console_newline();

    // CPU info - compact
    console_println_color("--- CPU Information ---", CONSOLE_HEADER_COLOR);
    console_print_color("Vendor: ", CONSOLE_INFO_COLOR);
    console_print_color(cpu_info.vendor, CONSOLE_FG_COLOR);
    console_print_color("  Family: ", CONSOLE_INFO_COLOR);
    int_to_str(cpu_info.family, buffer);
    console_print_color(buffer, CONSOLE_FG_COLOR);
    console_print_color("  Model: ", CONSOLE_INFO_COLOR);
    int_to_str(cpu_info.model, buffer);
    console_print_color(buffer, CONSOLE_FG_COLOR);
    console_print_color("  Stepping: ", CONSOLE_INFO_COLOR);
    int_to_str(cpu_info.stepping, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_print_color("Features: ", CONSOLE_INFO_COLOR);
    if (cpu_info.has_fpu) console_print_color("FPU ", CONSOLE_SUCCESS_COLOR);
    if (cpu_info.has_apic) console_print_color("APIC ", CONSOLE_SUCCESS_COLOR);
    if (cpu_info.has_sse) console_print_color("SSE ", CONSOLE_SUCCESS_COLOR);
    if (cpu_info.has_sse2) console_print_color("SSE2 ", CONSOLE_SUCCESS_COLOR);
    if (cpu_info.has_sse3) console_print_color("SSE3 ", CONSOLE_SUCCESS_COLOR);
    if (cpu_info.has_avx) console_print_color("AVX", CONSOLE_SUCCESS_COLOR);
    console_newline();

    console_newline();

    // Memory info - compact
    console_println_color("--- Memory Information ---", CONSOLE_HEADER_COLOR);
    uint64_t total_mem = multiboot2_get_total_memory();
    format_memory_size(total_mem, buffer, sizeof(buffer));
    console_print_color("Total Memory: ", CONSOLE_INFO_COLOR);
    console_print_color(buffer, CONSOLE_FG_COLOR);
    
    uint32_t mem_lower = multiboot2_get_memory_lower();
    console_print_color("  Lower: ", CONSOLE_INFO_COLOR);
    int_to_str(mem_lower, buffer);
    console_print_color(buffer, CONSOLE_FG_COLOR);
    console_print(" KB");
    
    uint32_t mem_upper = multiboot2_get_memory_upper();
    console_print_color("  Upper: ", CONSOLE_INFO_COLOR);
    int_to_str(mem_upper, buffer);
    console_print_color(buffer, CONSOLE_FG_COLOR);
    console_println(" KB");

    console_newline();

    // Display info - compact
    console_println_color("--- Display Information ---", CONSOLE_HEADER_COLOR);
    console_print_color("Mode: ", CONSOLE_INFO_COLOR);
    console_print_color("VGA Text Mode", CONSOLE_FG_COLOR);
    console_print_color("  Resolution: ", CONSOLE_INFO_COLOR);
    console_println_color("80x25", CONSOLE_FG_COLOR);

    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

/*
 * Pop function - displays compact system info in status bar area
 * This is called periodically from the main loop
 */
void sysinfo_pop_func(unsigned int start_pos) {
    (void)start_pos;  // Unused parameter
    
    // Detect CPU on first call
    if (!cpu_info_initialized) {
        sysinfo_detect_cpu();
    }
    
    // This pop module doesn't need to display anything in the main loop
    // The sysinfo command will call sysinfo_print_full() directly
}

/*
 * Pop module definition
 */
const PopModule sysinfo_module = {
    .name = "sysinfo",
    .message = "System information detection",
    .pop_function = sysinfo_pop_func
};

