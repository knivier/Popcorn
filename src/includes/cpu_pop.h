// src/includes/cpu_pop.h
#ifndef CPU_POP_H
#define CPU_POP_H

#include "pop_module.h"
#include <stdint.h>
#include <stdbool.h>

// CPU frequency information
typedef struct {
    uint64_t tsc_hz;           // TSC frequency in Hz (estimated)
    uint32_t mhz;              // Frequency in MHz
    bool frequency_detected;
} CPUFrequency;

// Extended CPU information
typedef struct {
    char vendor[13];
    char brand_string[49];     // Extended brand string from CPUID
    uint32_t family;
    uint32_t model;
    uint32_t stepping;
    uint32_t cores;            // Number of logical processors
    bool has_fpu;
    bool has_sse;
    bool has_sse2;
    bool has_sse3;
    bool has_ssse3;
    bool has_sse41;
    bool has_sse42;
    bool has_avx;
    bool has_avx2;
    bool has_apic;
    bool has_tsc;
    bool has_msr;
} ExtendedCPUInfo;

// External assembly functions
extern void cpuid_get_vendor(char* vendor_string);
extern void cpuid_get_features(uint32_t* output);
extern uint64_t rdtsc(void);
extern void cpuid_extended_brand(uint32_t leaf, uint32_t* output);

// Function declarations
void cpu_pop_func(unsigned int start_pos);
void cpu_detect_extended(void);
void cpu_detect_frequency(void);
void cpu_print_info(void);
void cpu_print_frequency(void);
const ExtendedCPUInfo* cpu_get_extended_info(void);
const CPUFrequency* cpu_get_frequency(void);

// Module definition
extern const PopModule cpu_module;

#endif // CPU_POP_H

