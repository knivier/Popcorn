#include "../includes/irq.h"
#include <stddef.h>

extern void write_port(unsigned short port, unsigned char data);
extern char read_port(unsigned short port);
extern void timer_handler(void);
extern void keyboard_handler(void);

#define INTERRUPT_GATE 0x8eU
#define PIC1_CMD 0x20
#define PIC1_DATA 0x21
#define PIC2_CMD 0xA0
#define PIC2_DATA 0xA1
#define PIC_EOI 0x20
#define PIC_READ_ISR 0x0B

static irq_handler_t g_irq_handlers[IRQ_COUNT];
static uint32_t g_unexpected_irq;
static uint32_t g_spurious_irq7;
static uint32_t g_spurious_irq15;
/* >0: timer must EOI but must NOT run the scheduler (USB/xHCI critical sections). */
static volatile int g_irq_quiet;

/* Known asm entry points for early IRQs; later IRQs can add stubs here. */
static void (*const g_irq_stubs[IRQ_COUNT])(void) = {
    [0] = timer_handler,
    [1] = keyboard_handler,
};

/* Port 0x80 delay — lets ancient PIC/ISA bridges settle between ICWs. */
static void io_wait(void) {
    write_port(0x80, 0);
}

void irq_init(void) {
    for (uint8_t i = 0; i < IRQ_COUNT; i++) {
        g_irq_handlers[i] = NULL;
    }
    g_unexpected_irq = 0;
    g_spurious_irq7 = 0;
    g_spurious_irq15 = 0;
    g_irq_quiet = 0;
}

void irq_quiet_enter(void) {
    g_irq_quiet++;
}

void irq_quiet_leave(void) {
    if (g_irq_quiet > 0) {
        g_irq_quiet--;
    }
}

int irq_quiet_active(void) {
    return g_irq_quiet > 0;
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
        uint8_t mask = (uint8_t)read_port(PIC1_DATA);
        write_port(PIC1_DATA, (unsigned char)(mask & (uint8_t)~(1U << irq)));
    } else {
        /* Slave IRQ: cascade line IRQ2 on the master must be unmasked. */
        uint8_t m1 = (uint8_t)read_port(PIC1_DATA);
        write_port(PIC1_DATA, (unsigned char)(m1 & (uint8_t)~(1U << 2)));
        uint8_t m2 = (uint8_t)read_port(PIC2_DATA);
        write_port(PIC2_DATA, (unsigned char)(m2 & (uint8_t)~(1U << (irq - 8))));
    }
}

void irq_disable(uint8_t irq) {
    if (irq >= IRQ_COUNT) {
        return;
    }
    if (irq < 8) {
        uint8_t mask = (uint8_t)read_port(PIC1_DATA);
        write_port(PIC1_DATA, (unsigned char)(mask | (uint8_t)(1U << irq)));
    } else {
        uint8_t mask = (uint8_t)read_port(PIC2_DATA);
        write_port(PIC2_DATA, (unsigned char)(mask | (uint8_t)(1U << (irq - 8))));
    }
}

void pic_send_eoi(uint8_t irq) {
    if (irq >= 8) {
        write_port(PIC2_CMD, PIC_EOI);
    }
    write_port(PIC1_CMD, PIC_EOI);
}

static int pic_isr_set(uint8_t irq) {
    if (irq < 8) {
        write_port(PIC1_CMD, PIC_READ_ISR);
        return (read_port(PIC1_CMD) & (1 << irq)) != 0;
    }
    write_port(PIC2_CMD, PIC_READ_ISR);
    return (read_port(PIC2_CMD) & (1 << (irq - 8))) != 0;
}

void irq_dispatch(uint8_t irq) {
    /* Spurious IRQ7 / IRQ15: ISR bit clear means no real request — do not EOI slave. */
    if (irq == 7 && !pic_isr_set(7)) {
        g_spurious_irq7++;
        return;
    }
    if (irq == 15 && !pic_isr_set(15)) {
        g_spurious_irq15++;
        /* Still EOI master (cascade); never EOI slave for spurious 15. */
        write_port(PIC1_CMD, PIC_EOI);
        return;
    }

    if (irq < IRQ_COUNT && g_irq_handlers[irq]) {
        g_irq_handlers[irq]();
        return;
    }
    g_unexpected_irq++;
    pic_send_eoi(irq);
}

uint32_t irq_unexpected_count(void) {
    return g_unexpected_irq;
}

uint32_t irq_spurious7_count(void) {
    return g_spurious_irq7;
}

uint32_t irq_spurious15_count(void) {
    return g_spurious_irq15;
}

void pic_init_remap(void) {
    uint8_t mask1 = (uint8_t)read_port(PIC1_DATA);
    uint8_t mask2 = (uint8_t)read_port(PIC2_DATA);

    write_port(PIC1_CMD, 0x11);
    io_wait();
    write_port(PIC2_CMD, 0x11);
    io_wait();

    write_port(PIC1_DATA, 0x20); /* master vectors 0x20–0x27 */
    io_wait();
    write_port(PIC2_DATA, 0x28); /* slave vectors 0x28–0x2F */
    io_wait();

    /* ICW3: master has slave on IRQ2; slave identity = 2 */
    write_port(PIC1_DATA, 0x04);
    io_wait();
    write_port(PIC2_DATA, 0x02);
    io_wait();

    write_port(PIC1_DATA, 0x01);
    io_wait();
    write_port(PIC2_DATA, 0x01);
    io_wait();

    /* Restore prior masks (or all-masked if first boot). */
    write_port(PIC1_DATA, mask1);
    write_port(PIC2_DATA, mask2);
}
