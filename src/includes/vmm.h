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

typedef struct {
    uint64_t pml4_phys;
} AddressSpace;

/*
 * Process PML4 policy (Linux-style split):
 *   lower 256 entries — unique per process (user code/stack/heap; start empty)
 *   upper 256 entries — shared copy of the kernel master PML4 (direct map, MMIO)
 *
 * Kernel walks page tables via the direct map (phys_to_virt), not raw identity.
 */

void vmm_init(void);

uint64_t vmm_alloc_pml4(void);

/*
 * Build a fresh kernel-capable layout on an empty PML4: identity + direct map
 * for the full boot window (VMM_IDENTITY_GIB). Prefer vmm_clone_kernel_space
 * for processes so they share the master upper half.
 */
int vmm_map_kernel_region(uint64_t pml4_phys);

/*
 * Copy upper 256 PML4 entries from src (master) into dst; leave lower 256 clear.
 * Shares kernel PDPT/PD pages — do not free those from a process teardown.
 */
int vmm_clone_kernel_space(uint64_t dst_pml4_phys, uint64_t src_pml4_phys);

/*
 * New process root: clone upper half from kernel_reference (or current CR3 if 0).
 */
int vmm_init_process_address_space(uint64_t process_pml4_phys, uint64_t kernel_reference_pml4_phys);

int vmm_map_4k(uint64_t pml4_phys, uint64_t vaddr, uint64_t paddr, uint64_t flags);
int vmm_unmap_4k(uint64_t pml4_phys, uint64_t vaddr);

void vmm_invalidate_page(uintptr_t vaddr);
void vmm_load_cr3(uint64_t pml4_phys);
uint64_t vmm_get_cr3(void);

/* Direct-map helpers (aliases of physmap.h for callers that include only vmm.h). */
#define vmm_phys_to_virt(phys) phys_to_virt(phys)

#endif
