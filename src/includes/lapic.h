#ifndef LAPIC_H
#define LAPIC_H

#include <stdbool.h>
#include <stdint.h>

/*
 * Local APIC "virtual wire" setup so 8259 PIC interrupts reach the CPU on every boot
 * path. UEFI firmware leaves the LAPIC enabled and may leave LINT0 masked and its own
 * LAPIC timer / IOAPIC routes armed; then IRQ0 never arrives (the old "sti;hlt hangs"
 * on UEFI). After this: LINT0 = ExtINT (PIC), LINT1 = NMI, LAPIC timer and error LVTs
 * masked, spurious vector 0xFF, TPR 0, and every IOAPIC redirection entry masked.
 * The LAPIC timer / MSI path (virtio-net M0) builds on the same mapping later.
 */
void lapic_setup_virtual_wire(void);
/* True when an enabled LAPIC was found (and programmed). */
bool lapic_active(void);
/* EOI to the LAPIC (no-op when there is none). */
void lapic_eoi(void);
/* Fixed-delivery IPI to this CPU on `vector`; -1 without a LAPIC. */
int lapic_self_ipi(uint8_t vector);
uint32_t lapic_id(void);
uint64_t lapic_base_phys(void);
/* IOAPIC redirection entries masked by lapic_setup_virtual_wire (0 if none found). */
uint32_t ioapic_masked_entries(void);

uint32_t isr_unexpected_vector_count(void);
uint32_t isr_lapic_spurious_count(void);

#endif /* LAPIC_H */
