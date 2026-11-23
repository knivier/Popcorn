#ifndef POPCORN_PHYSMAP_H
#define POPCORN_PHYSMAP_H

#include <stdint.h>
#include <stddef.h>

/*
 * Physical / direct-map layout — keep in sync with kernel.asm:
 *   NUM_L2_IDENTITY = 64  →  64 × 512 × 2 MiB = 64 GiB identity + high-half.
 *
 * VA 0xFFFF800000000000 + phys  ↔  phys   (direct map / “physmap”)
 * VA phys (low)                 ↔  phys   (identity, same pages)
 */

#define VMM_DIRECT_MAP_BASE   0xFFFF800000000000ULL

/* Boot page tables cover this many physical bytes (identity + high-half). */
#define VMM_IDENTITY_GIB      64u
#define VMM_IDENTITY_BYTES    ((uint64_t)VMM_IDENTITY_GIB << 30)

/*
 * The PMM (core/memory.c) sizes its frame bitmap at boot to the highest
 * available RAM in the memory map, capped at VMM_IDENTITY_BYTES.
 */

static inline void* phys_to_virt(uint64_t phys) {
    return (void*)(uintptr_t)(VMM_DIRECT_MAP_BASE + phys);
}

static inline uint64_t virt_to_phys_direct(const void* virt) {
    uint64_t v = (uint64_t)(uintptr_t)virt;
    if (v >= VMM_DIRECT_MAP_BASE && v < VMM_DIRECT_MAP_BASE + VMM_IDENTITY_BYTES) {
        return v - VMM_DIRECT_MAP_BASE;
    }
    /* Identity low pointer (kmalloc / early boot). */
    return v;
}

#endif /* POPCORN_PHYSMAP_H */
