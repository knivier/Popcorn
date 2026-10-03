// src/core/vmm.c — 4-level map; table walks via direct map; process PML4 split
#include "../includes/vmm.h"
#include "../includes/memory.h"
#include <stddef.h>
#include <stdint.h>

#define VMM_PDE_2M (VMM_PTE_P | VMM_PTE_RW | (1ull << 7))
#define VMM_2M_COUNT 512u /* 512 × 2 MiB = 1 GiB per PD */

#define EFER_MSR 0xC0000080u
#define EFER_NXE (1u << 11)

#define PD_MASK 0x1FFull
#define PTE_ADDR_MASK 0x000ffffffffff000ull

static inline uint32_t pml4_i(uint64_t v) { return (uint32_t)((v >> 39) & PD_MASK); }
static inline uint32_t pdpt_i(uint64_t v) { return (uint32_t)((v >> 30) & PD_MASK); }
static inline uint32_t pd_i(uint64_t v) { return (uint32_t)((v >> 21) & PD_MASK); }
static inline uint32_t pt_i(uint64_t v) { return (uint32_t)((v >> 12) & PD_MASK); }

/* Page-table pages are physical; access them through the direct map. */
static inline uint64_t* vmm_phys_to_ptr(uint64_t phys) {
    return (uint64_t*)phys_to_virt(phys);
}

#define TABLE_ENT (VMM_PTE_P | VMM_PTE_RW)

static int vmm_ensure_subtable(uint64_t* table, uint32_t index) {
    if (table[index] & VMM_PTE_P) {
        /* Huge PDE — cannot install a PT underneath without a splitter. */
        if (table[index] & (1ull << 7)) {
            return -3;
        }
        return 0;
    }
    void* p = alloc_pages(1, MEM_ALLOC_ZERO);
    if (!p) {
        return -1;
    }
    uint64_t phys = virt_to_phys_direct(p);
    table[index] = phys | TABLE_ENT;
    return 0;
}

void vmm_init(void) {
    uint32_t lo;
    uint32_t hi;
    __asm__ volatile("rdmsr" : "=a"(lo), "=d"(hi) : "c"((uint32_t)EFER_MSR));
    if ((lo & EFER_NXE) == 0U) {
        lo |= EFER_NXE;
        __asm__ volatile("wrmsr" : : "a"(lo), "d"(hi), "c"((uint32_t)EFER_MSR) : "memory");
    }
}

uint64_t vmm_alloc_pml4(void) {
    void* p = alloc_pages(1, MEM_ALLOC_ZERO);
    if (!p) {
        return 0;
    }
    return virt_to_phys_direct(p);
}

/*
 * Fill one PD with 512 × 2 MiB identity entries starting at phys_base.
 */
static void vmm_fill_pd_2m(uint64_t* pd, uint64_t phys_base) {
    for (uint32_t i = 0; i < VMM_2M_COUNT; i++) {
        pd[i] = (phys_base + (uint64_t)i * (2ull * 1024ull * 1024ull)) | VMM_PDE_2M;
    }
}

int vmm_map_kernel_region(uint64_t pml4_phys) {
    if ((pml4_phys & (PAGE_SIZE - 1U)) != 0) {
        return -2;
    }
    uint64_t* pml4 = vmm_phys_to_ptr(pml4_phys);
    if (pml4[0] != 0 || pml4[256] != 0) {
        return -3;
    }

    void* p_pdpt_lo = alloc_pages(1, MEM_ALLOC_ZERO);
    void* p_pdpt_hi = alloc_pages(1, MEM_ALLOC_ZERO);
    if (!p_pdpt_lo || !p_pdpt_hi) {
        return -1;
    }
    uint64_t pdpt_lo = virt_to_phys_direct(p_pdpt_lo);
    uint64_t pdpt_hi = virt_to_phys_direct(p_pdpt_hi);
    pml4[0] = pdpt_lo | TABLE_ENT;
    pml4[256] = pdpt_hi | TABLE_ENT;

    uint64_t* lo = vmm_phys_to_ptr(pdpt_lo);
    uint64_t* hi = vmm_phys_to_ptr(pdpt_hi);

    for (uint32_t g = 0; g < VMM_PDPT_SLOTS; g++) {
        void* p_pd = alloc_pages(1, MEM_ALLOC_ZERO);
        if (!p_pd) {
            return -1;
        }
        uint64_t pd_phys = virt_to_phys_direct(p_pd);
        lo[g] = pd_phys | TABLE_ENT;
        hi[g] = pd_phys | TABLE_ENT; /* share PD: identity and direct map */
        vmm_fill_pd_2m(vmm_phys_to_ptr(pd_phys), (uint64_t)g << 30);
    }
    return 0;
}

int vmm_clone_kernel_space(uint64_t dst_pml4_phys, uint64_t src_pml4_phys) {
    if ((dst_pml4_phys & (PAGE_SIZE - 1U)) != 0 || (src_pml4_phys & (PAGE_SIZE - 1U)) != 0) {
        return -2;
    }
    if (dst_pml4_phys == src_pml4_phys) {
        return 0;
    }
    uint64_t* dst = vmm_phys_to_ptr(dst_pml4_phys);
    const uint64_t* src = vmm_phys_to_ptr(src_pml4_phys);

    /* User half stays empty (unique per process). */
    for (uint32_t i = 0; i < 256u; i++) {
        dst[i] = 0;
    }
    /* Kernel half: share master upper entries (direct map, MMIO, kernel VA). */
    for (uint32_t i = 256u; i < 512u; i++) {
        dst[i] = src[i];
    }
    return 0;
}

int vmm_init_process_address_space(uint64_t process_pml4_phys, uint64_t kernel_reference_pml4_phys) {
    if (process_pml4_phys == 0) {
        return -4;
    }
    uint64_t master = kernel_reference_pml4_phys;
    if (master == 0) {
        master = vmm_get_cr3() & PTE_ADDR_MASK;
    }
    return vmm_clone_kernel_space(process_pml4_phys, master);
}

int vmm_map_4k(uint64_t pml4_phys, uint64_t vaddr, uint64_t paddr, uint64_t flags) {
    if ((pml4_phys & (PAGE_SIZE - 1U)) != 0 || (vaddr & (PAGE_SIZE - 1U)) != 0 ||
        (paddr & (PAGE_SIZE - 1U)) != 0) {
        return -2;
    }
    if ((flags & VMM_PTE_P) == 0) {
        return -2;
    }

    uint64_t* pml4 = vmm_phys_to_ptr(pml4_phys);
    int rc = vmm_ensure_subtable(pml4, pml4_i(vaddr));
    if (rc != 0) {
        return rc;
    }
    uint64_t pdpt_phys = pml4[pml4_i(vaddr)] & PTE_ADDR_MASK;
    uint64_t* pdpt = vmm_phys_to_ptr(pdpt_phys);

    rc = vmm_ensure_subtable(pdpt, pdpt_i(vaddr));
    if (rc != 0) {
        return rc;
    }
    uint64_t pd_phys = pdpt[pdpt_i(vaddr)] & PTE_ADDR_MASK;
    uint64_t* pd = vmm_phys_to_ptr(pd_phys);

    rc = vmm_ensure_subtable(pd, pd_i(vaddr));
    if (rc != 0) {
        return rc; /* -3 = need PDE splitter over a 2 MiB leaf */
    }
    uint64_t pt_phys = pd[pd_i(vaddr)] & PTE_ADDR_MASK;
    uint64_t* pt = vmm_phys_to_ptr(pt_phys);

    uint64_t leaf = (paddr & PTE_ADDR_MASK) | (flags & 0xFFF) | (flags & VMM_PTE_NX);
    pt[pt_i(vaddr)] = leaf;
    vmm_invalidate_page((uintptr_t)vaddr);
    return 0;
}

int vmm_unmap_4k(uint64_t pml4_phys, uint64_t vaddr) {
    if ((pml4_phys & (PAGE_SIZE - 1U)) != 0 || (vaddr & (PAGE_SIZE - 1U)) != 0) {
        return -2;
    }
    uint64_t* pml4 = vmm_phys_to_ptr(pml4_phys);
    uint32_t i4 = pml4_i(vaddr);
    if ((pml4[i4] & VMM_PTE_P) == 0) {
        return 0;
    }
    uint64_t* pdpt = vmm_phys_to_ptr(pml4[i4] & PTE_ADDR_MASK);
    uint32_t i3 = pdpt_i(vaddr);
    if ((pdpt[i3] & VMM_PTE_P) == 0) {
        return 0;
    }
    uint64_t* pd = vmm_phys_to_ptr(pdpt[i3] & PTE_ADDR_MASK);
    uint32_t i2 = pd_i(vaddr);
    if ((pd[i2] & VMM_PTE_P) == 0) {
        return 0;
    }
    if (pd[i2] & (1ull << 7)) {
        return -3; /* huge page — need splitter */
    }
    uint64_t* pt = vmm_phys_to_ptr(pd[i2] & PTE_ADDR_MASK);
    pt[pt_i(vaddr)] = 0;
    vmm_invalidate_page((uintptr_t)vaddr);
    return 0;
}

void vmm_invalidate_page(uintptr_t vaddr) {
    uintptr_t a = vaddr;
    __asm__ volatile("invlpg (%0)" : : "r"(a) : "memory", "cc");
}

void vmm_load_cr3(uint64_t pml4_phys) {
    __asm__ volatile("mov %0, %%cr3" : : "r"(pml4_phys) : "memory");
}

uint64_t vmm_get_cr3(void) {
    uint64_t c;
    __asm__ volatile("mov %%cr3, %0" : "=r"(c));
    return c;
}
