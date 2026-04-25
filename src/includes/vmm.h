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
 * Long-term memory policy (target): kernel invariant in the upper canonical
 * range, user in the low canonical range, with no accidental identity/MMIO
 * in user PML4s. Today the boot path still uses a low identity map, so
 * "kernel visibility" and bootstrap machinery are tied to the same PML4 slot.
 * vmm_init_process_address_space() is the policy entry; vmm_clone_kernel_space()
 * is the bootstrap implementation until explicit high-half (or per-range)
 * injection replaces it.
 */

/* Call once after physmem is up; enables EFER.NXE so VMM_PTE_NX is legal. */
void vmm_init(void);

/* New empty PML4 (one frame, zeroed). Returns 0 on failure. */
uint64_t vmm_alloc_pml4(void);

/*
 * Share kernel page-table structure into an empty (or prepared) PML4.
 * Copies PML4[i] from src to dst for fixed kernel slot indices. Subtables
 * (PDPT/PD/PT) are shared: same machine pointers, no deep copy of frames.
 *
 * Invariant: any PML4 loaded for a task must lead to the same *kernel* VA
 * mappings as the boot root, or syscalls/IRQ/IRET will miss kernel code.
 *
 * Current layout: boot identity (1 GiB) lives under PML4 slot 0 (first
 * 512 GiB of canonical low addresses). A future high-half kernel would add
 * more slot indices. Note: sharing slot 0 shares the *entire* subtree under
 * it between roots; for multiple *isolated* user spaces, plan either a
 * high-half kernel + user-only low tables, or stop sharing slot 0 and
 * install kernel mappings explicitly per policy.
 *
 * Returns: 0 success, -1 missing kernel slot in src, -2 invalid alignment,
 * -3 dst slot already maps a different subtree.
 */
int vmm_clone_kernel_space(uint64_t dst_pml4_phys, uint64_t src_pml4_phys);

/*
 * Prepare a *process* PML4: ensure kernel is reachable the same way as
 * `kernel_reference_pml4` (e.g. scheduler_kernel_pml4_phys()). `process_pml4`
 * should be a fresh vmm_alloc_pml4() root. User mappings stay empty;
 * vmm_map_4k() fills the user portion later.
 *
 * Current implementation: delegates to vmm_clone_kernel_space (duplicates
 * "kernel" slots from the reference root). Future: replace with explicit
 * invariant kernel mappings and no boot-identity over-share for user space.
 * Return codes same as vmm_clone_kernel_space; -4 if process_pml4 is 0.
 */
int vmm_init_process_address_space(uint64_t process_pml4_phys, uint64_t kernel_reference_pml4_phys);

/* Map one 4 KiB page. pml4 is the physical address of the root. */
int vmm_map_4k(uint64_t pml4_phys, uint64_t vaddr, uint64_t paddr, uint64_t flags);

int vmm_unmap_4k(uint64_t pml4_phys, uint64_t vaddr);

void vmm_invalidate_page(uintptr_t vaddr);
void vmm_load_cr3(uint64_t pml4_phys);
uint64_t vmm_get_cr3(void);

#endif
