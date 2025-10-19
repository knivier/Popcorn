// src/cpu_pop.c
#include "includes/cpu_pop.h"
#include "includes/console.h"
#include <stddef.h>
#include <stdbool.h>

// Static storage
static ExtendedCPUInfo cpu_extended = {0};
static CPUFrequency cpu_freq = {0};
static bool cpu_extended_initialized = false;
static bool cpu_freq_initialized = false;

// Access console state
extern ConsoleState console_state;

// External function to convert int to string
extern void int_to_str(int num, char *str);

// Delay function
extern void util_delay(unsigned int milliseconds);

// Detect extended CPU information
void cpu_detect_extended(void) {
    if (cpu_extended_initialized) {
        return;
    }
    
    // Get vendor string
    cpuid_get_vendor(cpu_extended.vendor);
    cpu_extended.vendor[12] = '\0';
    
    // Get basic features
    uint32_t cpuid_data[4];
    cpuid_get_features(cpuid_data);
    
    // Parse processor version info (EAX)
    uint32_t eax = cpuid_data[0];
    cpu_extended.stepping = eax & 0xF;
    cpu_extended.model = (eax >> 4) & 0xF;
    cpu_extended.family = (eax >> 8) & 0xF;
    
    // Extended model and family
    if (cpu_extended.family == 0xF) {
        cpu_extended.family += (eax >> 20) & 0xFF;
    }
    if (cpu_extended.family == 0x6 || cpu_extended.family == 0xF) {
        cpu_extended.model += ((eax >> 16) & 0xF) << 4;
    }
    
    // Parse feature flags
    uint32_t edx = cpuid_data[3];
    uint32_t ecx = cpuid_data[2];
    uint32_t ebx = cpuid_data[1];
    
    cpu_extended.has_fpu = (edx & (1 << 0)) != 0;
    cpu_extended.has_tsc = (edx & (1 << 4)) != 0;
    cpu_extended.has_msr = (edx & (1 << 5)) != 0;
    cpu_extended.has_apic = (edx & (1 << 9)) != 0;
    cpu_extended.has_sse = (edx & (1 << 25)) != 0;
    cpu_extended.has_sse2 = (edx & (1 << 26)) != 0;
    cpu_extended.has_sse3 = (ecx & (1 << 0)) != 0;
    cpu_extended.has_ssse3 = (ecx & (1 << 9)) != 0;
    cpu_extended.has_sse41 = (ecx & (1 << 19)) != 0;
    cpu_extended.has_sse42 = (ecx & (1 << 20)) != 0;
    cpu_extended.has_avx = (ecx & (1 << 28)) != 0;
    
    // Get logical processor count from EBX bits 16-23
    cpu_extended.cores = (ebx >> 16) & 0xFF;
    if (cpu_extended.cores == 0) {
        cpu_extended.cores = 1;
    }
    
    // Try to get brand string (CPUID leaves 0x80000002-0x80000004)
    cpu_extended.brand_string[0] = '\0';
    
    // Check if extended CPUID is available
    uint32_t ext_check[4];
    cpuid_extended_brand(0x80000000, ext_check);
    if (ext_check[0] >= 0x80000004) {
        // Get brand string in 3 parts
        uint32_t* brand_ptr = (uint32_t*)cpu_extended.brand_string;
        cpuid_extended_brand(0x80000002, brand_ptr);
        cpuid_extended_brand(0x80000003, brand_ptr + 4);
        cpuid_extended_brand(0x80000004, brand_ptr + 8);
        cpu_extended.brand_string[48] = '\0';
        
        // Trim leading spaces
        char* start = cpu_extended.brand_string;
        while (*start == ' ') start++;
        if (start != cpu_extended.brand_string) {
            int i = 0;
            while (start[i] != '\0') {
                cpu_extended.brand_string[i] = start[i];
                i++;
            }
            cpu_extended.brand_string[i] = '\0';
        }
    }
    
    // Check for AVX2 if AVX is present
    if (cpu_extended.has_avx) {
        uint32_t cpuid7[4];
        cpuid_extended_brand(7, cpuid7);
        cpu_extended.has_avx2 = (cpuid7[1] & (1 << 5)) != 0;
    } else {
        cpu_extended.has_avx2 = false;
    }
    
    cpu_extended_initialized = true;
}

// Detect CPU frequency using TSC
void cpu_detect_frequency(void) {
    if (cpu_freq_initialized) {
        return;
    }
    
    // Ensure extended info is available
    if (!cpu_extended_initialized) {
        cpu_detect_extended();
    }
    
    // Check if TSC is available
    if (!cpu_extended.has_tsc) {
        cpu_freq.frequency_detected = false;
        cpu_freq_initialized = true;
        return;
    }
    
    // Measure TSC ticks over ~100ms delay
    uint64_t tsc_start = rdtsc();
    util_delay(100);  // 100ms delay
    uint64_t tsc_end = rdtsc();
    
    uint64_t tsc_delta = tsc_end - tsc_start;
    
    // Calculate frequency: delta ticks / 0.1 seconds = Hz
    cpu_freq.tsc_hz = tsc_delta * 10;
    cpu_freq.mhz = (uint32_t)(cpu_freq.tsc_hz / 1000000);
    cpu_freq.frequency_detected = true;
    cpu_freq_initialized = true;
}

// Print CPU info
void cpu_print_info(void) {
    char buffer[128];
    
    if (!cpu_extended_initialized) {
        cpu_detect_extended();
    }
    
    console_newline();
    console_println_color("=== CPU INFORMATION ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    console_print_color("Vendor: ", CONSOLE_INFO_COLOR);
    console_println_color(cpu_extended.vendor, CONSOLE_SUCCESS_COLOR);
    
    if (cpu_extended.brand_string[0] != '\0') {
        console_print_color("Brand:  ", CONSOLE_INFO_COLOR);
        console_println_color(cpu_extended.brand_string, CONSOLE_FG_COLOR);
    }
    
    console_print_color("Family: ", CONSOLE_INFO_COLOR);
    int_to_str(cpu_extended.family, buffer);
    console_print_color(buffer, CONSOLE_FG_COLOR);
    console_print_color("  Model: ", CONSOLE_INFO_COLOR);
    int_to_str(cpu_extended.model, buffer);
    console_print_color(buffer, CONSOLE_FG_COLOR);
    console_print_color("  Stepping: ", CONSOLE_INFO_COLOR);
    int_to_str(cpu_extended.stepping, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_print_color("Cores:  ", CONSOLE_INFO_COLOR);
    int_to_str(cpu_extended.cores, buffer);
    console_println_color(buffer, CONSOLE_FG_COLOR);
    
    console_newline();
    console_println_color("Features:", CONSOLE_INFO_COLOR);
    console_print("  ");
    if (cpu_extended.has_fpu) console_print_color("FPU ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_tsc) console_print_color("TSC ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_msr) console_print_color("MSR ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_apic) console_print_color("APIC ", CONSOLE_SUCCESS_COLOR);
    console_newline();
    console_print("  ");
    if (cpu_extended.has_sse) console_print_color("SSE ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_sse2) console_print_color("SSE2 ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_sse3) console_print_color("SSE3 ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_ssse3) console_print_color("SSSE3 ", CONSOLE_SUCCESS_COLOR);
    console_newline();
    console_print("  ");
    if (cpu_extended.has_sse41) console_print_color("SSE4.1 ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_sse42) console_print_color("SSE4.2 ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_avx) console_print_color("AVX ", CONSOLE_SUCCESS_COLOR);
    if (cpu_extended.has_avx2) console_print_color("AVX2", CONSOLE_SUCCESS_COLOR);
    console_newline();
    
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

// Print CPU frequency
void cpu_print_frequency(void) {
    char buffer[64];
    
    if (!cpu_freq_initialized) {
        cpu_detect_frequency();
    }
    
    console_newline();
    console_println_color("=== CPU FREQUENCY ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    if (!cpu_freq.frequency_detected) {
        console_println_color("TSC not available - cannot measure frequency", CONSOLE_WARNING_COLOR);
    } else {
        console_print_color("Estimated: ", CONSOLE_INFO_COLOR);
        int_to_str(cpu_freq.mhz, buffer);
        console_print_color(buffer, CONSOLE_SUCCESS_COLOR);
        console_println(" MHz");
        
        console_print_color("TSC Rate:  ", CONSOLE_INFO_COLOR);
        // Format Hz in millions
        uint32_t mhz_tsc = (uint32_t)(cpu_freq.tsc_hz / 1000000);
        int_to_str(mhz_tsc, buffer);
        console_print_color(buffer, CONSOLE_FG_COLOR);
        console_println(" MHz");
        
        console_newline();
        console_println_color("Note: Frequency measured via TSC sampling", CONSOLE_INFO_COLOR);
    }
    
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

// Get extended CPU info pointer
const ExtendedCPUInfo* cpu_get_extended_info(void) {
    if (!cpu_extended_initialized) {
        cpu_detect_extended();
    }
    return &cpu_extended;
}

// Get frequency info pointer
const CPUFrequency* cpu_get_frequency(void) {
    if (!cpu_freq_initialized) {
        cpu_detect_frequency();
    }
    return &cpu_freq;
}

// Pop function
void cpu_pop_func(unsigned int start_pos) {
    (void)start_pos;
    
    // Initialize on first call
    if (!cpu_extended_initialized) {
        cpu_detect_extended();
    }
}

// Pop module definition
const PopModule cpu_module = {
    .name = "cpu",
    .message = "CPU detection and frequency monitoring",
    .pop_function = cpu_pop_func
};

