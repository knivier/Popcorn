#include "../includes/irq.h"
#include <stddef.h>

extern void write_port(unsigned short port, unsigned char data);
extern char read_port(unsigned short port);
extern void timer_handler(void);
extern void keyboard_handler(void);

#define INTERRUPT_GATE 0x8eU

static irq_handler_t g_irq_handlers[IRQ_COUNT];

/* Known asm entry points for early IRQs; later IRQs can add stubs here. */
static void (*const g_irq_stubs[IRQ_COUNT])(void) = {
    [0] = timer_handler,
    [1] = keyboard_handler,
};

void irq_init(void) {
    for (uint8_t i = 0; i < IRQ_COUNT; i++) {
        g_irq_handlers[i] = NULL;
    }
}

bool irq_register(uint8_t irq, irq_handler_t handler) {
    if (irq >= IRQ_COUNT || !handler || !g_irq_stubs[irq]) {
        return false;
    }
    g_irq_handlers[irq] = handler;
    idt_set_gate((uint8_t)(0x20U + irq), (uint64_t)(uintptr_t)g_irq_stubs[irq],
                 INTERRUPT_GATE, 0U);
    return true;
}

void irq_enable(uint8_t irq) {
    if (irq >= IRQ_COUNT) {
        return;
    }
    if (irq < 8) {
        uint8_t mask = (uint8_t)read_port(0x21);
        write_port(0x21, (unsigned char)(mask & (uint8_t)~(1U << irq)));
    } else {
        uint8_t mask = (uint8_t)read_port(0xA1);
        write_port(0xA1, (unsigned char)(mask & (uint8_t)~(1U << (irq - 8))));
    }
}

void irq_disable(uint8_t irq) {
    if (irq >= IRQ_COUNT) {
        return;
    }
    if (irq < 8) {
        uint8_t mask = (uint8_t)read_port(0x21);
        write_port(0x21, (unsigned char)(mask | (uint8_t)(1U << irq)));
    } else {
        uint8_t mask = (uint8_t)read_port(0xA1);
        write_port(0xA1, (unsigned char)(mask | (uint8_t)(1U << (irq - 8))));
    }
}

void irq_dispatch(uint8_t irq) {
    if (irq < IRQ_COUNT && g_irq_handlers[irq]) {
        g_irq_handlers[irq]();
        return;
    }
    /* Spurious / unregistered: EOI master (and slave if needed). */
    if (irq >= 8) {
        write_port(0xA0, 0x20);
    }
    write_port(0x20, 0x20);
}
