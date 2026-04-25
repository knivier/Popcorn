// src/includes/vmm.h — x86-64 virtual memory (4-level) on top of identity-mapped PMM
#ifndef VMM_H
#define VMM_H

#include <stdint.h>
#include <stddef.h>

/*
 * Page-table entry flags (leaf PTE and intermediate tables use P+RW for kernel).
 * Same layout as the CPU; combine with | for vmm_map_4k.
 */
#define VMM_PTE_P  (1ull << 0)  /* present */
#define VMM_PTE_RW (1ull << 1)  /* read/write; clear = read-only */
#define VMM_PTE_US (1ull << 2)  /* user/supervisor: set = user accessible */
#define VMM_PTE_NX (1ull << 63) /* execute disable (requires EFER.NXE) */

/* One active translation root per execution context; stored on each task. */
typedef struct {
    uint64_t pml4_phys;
} AddressSpace;

/*
 * Policy: process roots get an explicit layout (vmm_map_kernel_region), not a
 * clone of boot tables. Long-term target remains high-half kernel + user low;
 * PMM/kmalloc currently assume the first 1 GiB identity (see memory.c).
 */

/* Call once after physmem is up; enables EFER.NXE so VMM_PTE_NX is legal. */
void vmm_init(void);

/* New empty PML4 (one frame, zeroed). Returns 0 on failure. */
uint64_t vmm_alloc_pml4(void);

/*
 * Layout-driven kernel region: identity-map the first 1 GiB (VA 0 .. 1GiB-1)
 * to the same physical addresses using 512 × 2 MiB pages (same policy as
 * kernel.asm bootstrap). Allocates fresh PDPT and PD pages; does not copy
 * pointers from the boot PML4.
 *
 * Requires PML4 slot 0 clear. Covers kernel, framebuffer, and PMM-backed
 * heap frames in the first gigabyte. (vmm_map_4k in 0..1GiB on such a root
 * would require splitting a 2 MiB PDE; not implemented—use user VAs above 1 GiB
 * or add a splitter when you need 4 KiB overlays in the low gigabyte.)
 *
 * Returns: 0, -1 OOM, -2 bad pml4 alignment, -3 PML4[0] already used.
 */
int vmm_map_kernel_region(uint64_t pml4_phys);

/*
 * Legacy / migration: duplicate selected PML4 slots from an existing root
 * (shares PDPT/PD/PT physical pages). Prefer vmm_map_kernel_region for new
 * address spaces. Return codes: 0, -1..-3 as before.
 */
int vmm_clone_kernel_space(uint64_t dst_pml4_phys, uint64_t src_pml4_phys);

/*
 * Prepare a process PML4: apply kernel memory policy (vmm_map_kernel_region).
 * process_pml4 should be from vmm_alloc_pml4(). kernel_reference_pml4_phys is
 * unused (kept for API stability); previously used for clone-based init.
 * Returns: vmm_map_kernel_region errors, or -4 if process_pml4 is 0.
 */
int vmm_init_process_address_space(uint64_t process_pml4_phys, uint64_t kernel_reference_pml4_phys);

/* Map one 4 KiB page. pml4 is the physical address of the root. */
int vmm_map_4k(uint64_t pml4_phys, uint64_t vaddr, uint64_t paddr, uint64_t flags);

int vmm_unmap_4k(uint64_t pml4_phys, uint64_t vaddr);

void vmm_invalidate_page(uintptr_t vaddr);
void vmm_load_cr3(uint64_t pml4_phys);
uint64_t vmm_get_cr3(void);

#endif
