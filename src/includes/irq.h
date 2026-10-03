#ifndef IRQ_H
#define IRQ_H

#include <stdint.h>
#include <stdbool.h>

#define IRQ_COUNT 16

typedef void (*irq_handler_t)(void);

/* After PIC remap in idt_init: clear table, install IRQ0/1 stubs via register. */
void irq_init(void);

/* Install C handler for ISA IRQ 0–15 and wire IDT vector 0x20+irq. */
bool irq_register(uint8_t irq, irq_handler_t handler);

void irq_enable(uint8_t irq);
void irq_disable(uint8_t irq);

/* Asm IRQ stubs call this with the ISA IRQ number. */
void irq_dispatch(uint8_t irq);

/* Used by irq_register to install gates (implemented in kernel.c). */
void idt_set_gate(uint8_t vector, uint64_t handler, uint8_t type_attr, uint8_t ist);

#endif /* IRQ_H */
