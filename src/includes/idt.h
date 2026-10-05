#ifndef IDT_H
#define IDT_H

#include <stdint.h>

#define IDT_SIZE 256
#define INTERRUPT_GATE 0x8e
#define KERNEL_CODE_SEGMENT_OFFSET 0x08

void idt_set_gate(uint8_t vector, uint64_t handler, uint8_t type_attr, uint8_t ist);
void idt_init(void);

#endif /* IDT_H */
