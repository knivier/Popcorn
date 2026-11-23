#ifndef IRQ_H
#define IRQ_H

#include <stdint.h>
#include <stdbool.h>

#define IRQ_COUNT 16

typedef void (*irq_handler_t)(void);

/* Clear the handler table and install IDT gates for every PIC vector 0x20-0x2F
 * (generic idt.asm stubs), so spurious IRQ7/15 and unregistered IRQs reach
 * irq_dispatch instead of faulting on a not-present gate. */
void irq_init(void);

/* Remap PIC1/PIC2 to 0x20/0x28 with correct ICW3 cascade (master 0x04, slave 0x02). */
void pic_init_remap(void);

/* Install C handler for ISA IRQ 0–15 (gate for vector 0x20+irq is set by irq_init).
 * The handler owns the EOI for its IRQ; unhandled IRQs are EOI'd by irq_dispatch. */
bool irq_register(uint8_t irq, irq_handler_t handler);

void irq_enable(uint8_t irq);
void irq_disable(uint8_t irq);

/* Slave-before-master EOI. Spurious IRQ7/15 handled in irq_dispatch. */
void pic_send_eoi(uint8_t irq);

/* Asm IRQ stubs call this with the ISA IRQ number. */
void irq_dispatch(uint8_t irq);

/* Nested: timer EOIs but skips scheduler (USB/xHCI BOT critical sections). */
void irq_quiet_enter(void);
void irq_quiet_leave(void);
int irq_quiet_active(void);

uint32_t irq_unexpected_count(void);
uint32_t irq_spurious7_count(void);
uint32_t irq_spurious15_count(void);

/* Used by irq_init to install gates (implemented in idt.c). */
void idt_set_gate(uint8_t vector, uint64_t handler, uint8_t type_attr, uint8_t ist);

/* Save RFLAGS and disable interrupts; pair with irq_restore_flags(). Nestable. */
static inline uint64_t irq_save_disable(void) {
    uint64_t flags;
    __asm__ volatile("pushfq\n\tpopq %0\n\tcli" : "=r"(flags) : : "memory");
    return flags;
}

/* Re-enable interrupts only if they were enabled when the flags were saved. */
static inline void irq_restore_flags(uint64_t flags) {
    if (flags & 0x200u) {
        __asm__ volatile("sti" : : : "memory");
    }
}

#endif /* IRQ_H */
