// src/core/vmm.c — 4-level map; subtables from PMM (identity: phys == virt pointer)
#include "../includes/vmm.h"
#include "../includes/memory.h"
#include <stddef.h>
#include <stdint.h>

/*
 * PML4 indices whose entries must be replicated from the kernel (boot) root
 * into a new PML4 so the CPU can still run kernel code after task_set_address_space.
 * Only slot 0 for the current 512 GiB low region containing the 1 GiB identity map;
 * add 256..511 when the kernel is also mapped in the upper canonical range.
 */
static const uint32_t g_kernel_pml4_slot[] = {0u};

#define G_KERNEL_PML4_SLOT_LEN (sizeof g_kernel_pml4_slot / sizeof g_kernel_pml4_slot[0])

#define EFER_MSR 0xC0000080u
#define EFER_NXE (1u << 11)

#define PD_MASK 0x1FFull

static inline uint32_t pml4_i(uint64_t v) { return (uint32_t)((v >> 39) & PD_MASK); }
static inline uint32_t pdpt_i(uint64_t v) { return (uint32_t)((v >> 30) & PD_MASK); }
static inline uint32_t pd_i(uint64_t v) { return (uint32_t)((v >> 21) & PD_MASK); }
static inline uint32_t pt_i(uint64_t v) { return (uint32_t)((v >> 12) & PD_MASK); }

static inline uint64_t* vmm_phys_to_ptr(uint64_t phys) { return (uint64_t*)(uintptr_t)phys; }

/* Intermediate levels: P + RW, supervisor. */
#define TABLE_ENT (VMM_PTE_P | VMM_PTE_RW)

static int vmm_ensure_subtable(uint64_t* table, uint32_t index) {
    if (table[index] & VMM_PTE_P) {
        return 0;
    }
    void* p = alloc_pages(1, MEM_ALLOC_ZERO);
    if (!p) {
        return -1;
    }
    uint64_t phys = (uint64_t)(uintptr_t)p;
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
    return (uint64_t)(uintptr_t)p;
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
    for (size_t j = 0; j < G_KERNEL_PML4_SLOT_LEN; j++) {
        uint32_t i = g_kernel_pml4_slot[j];
        if (i >= 512u) {
            return -2;
        }
        if ((src[i] & VMM_PTE_P) == 0) {
            return -1;
        }
        if ((dst[i] & VMM_PTE_P) != 0 && (dst[i] & 0x000ffffffffff000ull) != (src[i] & 0x000ffffffffff000ull)) {
            return -3;
        }
        dst[i] = src[i];
    }
    return 0;
}

int vmm_init_process_address_space(uint64_t process_pml4_phys, uint64_t kernel_reference_pml4_phys) {
    if (process_pml4_phys == 0) {
        return -4;
    }
    return vmm_clone_kernel_space(process_pml4_phys, kernel_reference_pml4_phys);
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
    if (vmm_ensure_subtable(pml4, pml4_i(vaddr)) != 0) {
        return -1;
    }
    uint64_t pdpt_phys = pml4[pml4_i(vaddr)] & 0x000ffffffffff000ull;
    uint64_t* pdpt = vmm_phys_to_ptr(pdpt_phys);

    if (vmm_ensure_subtable(pdpt, pdpt_i(vaddr)) != 0) {
        return -1;
    }
    uint64_t pd_phys = pdpt[pdpt_i(vaddr)] & 0x000ffffffffff000ull;
    uint64_t* pd = vmm_phys_to_ptr(pd_phys);

    if (vmm_ensure_subtable(pd, pd_i(vaddr)) != 0) {
        return -1;
    }
    uint64_t pt_phys = pd[pd_i(vaddr)] & 0x000ffffffffff000ull;
    uint64_t* pt = vmm_phys_to_ptr(pt_phys);

    uint64_t leaf = (paddr & 0x000ffffffffff000ull) | (flags & 0xFFF) | (flags & VMM_PTE_NX);
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
    uint64_t* pdpt = vmm_phys_to_ptr(pml4[i4] & 0x000ffffffffff000ull);
    uint32_t i3 = pdpt_i(vaddr);
    if ((pdpt[i3] & VMM_PTE_P) == 0) {
        return 0;
    }
    uint64_t* pd = vmm_phys_to_ptr(pdpt[i3] & 0x000ffffffffff000ull);
    uint32_t i2 = pd_i(vaddr);
    if ((pd[i2] & VMM_PTE_P) == 0) {
        return 0;
    }
    uint64_t* pt = vmm_phys_to_ptr(pd[i2] & 0x000ffffffffff000ull);
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
