// src/includes/sysinfo_pop.h
#ifndef SYSINFO_POP_H
#define SYSINFO_POP_H

#include "pop_module.h"
#include <stdint.h>
#include <stdbool.h>

// CPU information structure
typedef struct {
    char vendor[13];         // 12 chars + null terminator
    uint32_t family;
    uint32_t model;
    uint32_t stepping;
    bool has_fpu;
    bool has_sse;
    bool has_sse2;
    bool has_sse3;
    bool has_avx;
    bool has_apic;
} CPUInfo;

// External CPUID functions (from core/kernel.asm)
extern void cpuid_get_vendor(char* vendor_string);
extern void cpuid_get_features(uint32_t* output);

// Function declarations
void sysinfo_pop_func(unsigned int start_pos);
void sysinfo_detect_cpu(void);
const CPUInfo* sysinfo_get_cpu_info(void);
void sysinfo_print_full(void);

// Module definition
extern const PopModule sysinfo_module;

#endif // SYSINFO_POP_H

