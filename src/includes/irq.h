#ifndef IRQ_H
#define IRQ_H

#include <stdint.h>
#include <stdbool.h>

#define IRQ_COUNT 16

typedef void (*irq_handler_t)(void);

/* After PIC remap in idt_init: clear table, install IRQ0/1 stubs via register. */
void irq_init(void);

/* Remap PIC1/PIC2 to 0x20/0x28 with correct ICW3 cascade (master 0x04, slave 0x02). */
void pic_init_remap(void);

/* Install C handler for ISA IRQ 0–15 and wire IDT vector 0x20+irq. */
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

/* Used by irq_register to install gates (implemented in idt.c). */
void idt_set_gate(uint8_t vector, uint64_t handler, uint8_t type_attr, uint8_t ist);

#endif /* IRQ_H */
