// src/includes/vmm.h — x86-64 virtual memory (4-level) + process PML4 split
#ifndef VMM_H
#define VMM_H

#include <stdint.h>
#include <stddef.h>
#include "physmap.h"

/*
 * Page-table entry flags (leaf PTE and intermediate tables use P+RW for kernel).
 */
#define VMM_PTE_P  (1ull << 0)
#define VMM_PTE_RW (1ull << 1)
#define VMM_PTE_US (1ull << 2)
#define VMM_PTE_NX (1ull << 63)

#define VMM_PTE_ADDR_MASK 0x000ffffffffff000ull

typedef struct {
    uint64_t pml4_phys;
} AddressSpace;

/*
 * Process PML4 policy:
 *   PML4[0]     — shared identity (kmalloc/stacks still use low VAs)
 *   PML4[1..255]— private (empty; future user maps)
 *   PML4[256..] — shared copy of kernel master (direct map / high-half)
 *
 * Table walks use the direct map (phys_to_virt).
 */

void vmm_init(void);

uint64_t vmm_alloc_pml4(void);
/* Free a private PML4 page only (never frees shared PDPT/PD). No-op if master. */
void vmm_free_pml4(uint64_t pml4_phys, uint64_t kernel_master_pml4_phys);

int vmm_map_kernel_region(uint64_t pml4_phys);

/*
 * Build a task root from master: share identity + kernel half; clear PML4[1..255].
 */
int vmm_clone_kernel_space(uint64_t dst_pml4_phys, uint64_t src_pml4_phys);

int vmm_init_process_address_space(uint64_t process_pml4_phys, uint64_t kernel_reference_pml4_phys);

int vmm_map_4k(uint64_t pml4_phys, uint64_t vaddr, uint64_t paddr, uint64_t flags);
int vmm_unmap_4k(uint64_t pml4_phys, uint64_t vaddr);

void vmm_invalidate_page(uintptr_t vaddr);
void vmm_flush_tlb(void);
void vmm_load_cr3(uint64_t pml4_phys);
uint64_t vmm_get_cr3(void);

#define vmm_phys_to_virt(phys) phys_to_virt(phys)

#endif
