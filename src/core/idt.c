#include "../includes/idt.h"
#include "../includes/irq.h"
#include "../includes/timer.h"
#include <stddef.h>
#include <stdint.h>

extern void keyboard_handler(void);
extern void timer_handler(void);
extern void default_cpu_exception(void);
extern void exc_double_fault(void);
extern void exc_general_protection(void);
extern void exc_page_fault(void);
extern void keyboard_handler_main(void);
extern char read_port(unsigned short port);
extern void write_port(unsigned short port, unsigned char data);
extern char stack_top;

/* 64-bit IDT entry structure (16 bytes) */
struct IDT_entry {
    unsigned short int offset_low;      /* offset bits 0..15 */
    unsigned short int selector;        /* code segment selector */
    unsigned char ist;                  /* bits 0..2 holds Interrupt Stack Table offset, rest reserved */
    unsigned char type_attr;            /* type and attributes */
    unsigned short int offset_mid;      /* offset bits 16..31 */
    unsigned int offset_high;           /* offset bits 32..63 */
    unsigned int reserved;              /* reserved */
} __attribute__((packed));

struct IDT_entry IDT[IDT_SIZE];

void idt_set_gate(uint8_t vector, uint64_t handler, uint8_t type_attr, uint8_t ist) {
    IDT[vector].offset_low = (uint16_t)(handler & 0xFFFFU);
    IDT[vector].selector = KERNEL_CODE_SEGMENT_OFFSET;
    IDT[vector].ist = (unsigned char)(ist & 0x7U);
    IDT[vector].type_attr = type_attr;
    IDT[vector].offset_mid = (uint16_t)((handler >> 16) & 0xFFFFU);
    IDT[vector].offset_high = (uint32_t)((handler >> 32) & 0xFFFFFFFFU);
    IDT[vector].reserved = 0;
}

struct GDT_ptr {
    uint16_t limit;
    uint64_t base;
} __attribute__((packed));

/* 64-bit TSS, IST1 = #PF, IST2 = #DF. TSS is a 16-byte GDT entry; it must be 16-byte aligned
   in the GDT (Intel SDM, IA-32e: misaligned 16B system descriptor + LTR = #GP → triple-fault/reset). */
#define GDT_TSS_SEL 0x20u /* GDT index 4 after null+code+data+pad — offset 0x20 from GDT base */
static void tss_ist_lgdt_ltr(void) {
    static uint8_t tss[104] __attribute__((aligned(16)));
    static uint8_t ist_pf[4096] __attribute__((aligned(16)));
    static uint8_t ist_df[4096] __attribute__((aligned(16)));
    /* 6*8: null, code, data, 8B pad, TSS (16B). Pad puts TSS at offset 0x20 (16B-aligned). */
    static uint64_t gdt6[6] __attribute__((aligned(32)));

    for (size_t i = 0; i < sizeof tss; i++) {
        tss[i] = 0;
    }
    const uint32_t tss_size = 104U;
    /* x86-64 TSS: RSP0@4, IST1@0x24, IST2@0x2C; I/O map base@0x66 — must be > limit or no I/O map */
    *(uint64_t*)(void*)(tss + 0x4) = (uint64_t)(uintptr_t)&stack_top;
    *(uint64_t*)(void*)(tss + 0x24) = (uint64_t)(uintptr_t)(ist_pf + sizeof ist_pf);
    *(uint64_t*)(void*)(tss + 0x2c) = (uint64_t)(uintptr_t)(ist_df + sizeof ist_df);
    *(uint16_t*)(void*)(tss + 0x66) = (uint16_t)tss_size; /* 0x68: no I/O perm bitmap (104 bytes) */

    const uint64_t tss_b = (uint64_t)(uintptr_t)tss;
    const uint32_t lim = tss_size - 1U;

    gdt6[0] = 0;
    gdt6[1] = 0x00209A0000000000ULL; /* match kernel.asm long-mode code */
    gdt6[2] = 0x0000920000000000ULL; /* data */
    gdt6[3] = 0;                     /* 8B padding: next slot at offset 0x20 */
    {
        uint8_t* d = (uint8_t*)&gdt6[4];
        d[0] = (uint8_t)(lim & 0xFFU);
        d[1] = (uint8_t)((lim >> 8) & 0xFFU);
        d[2] = (uint8_t)(tss_b & 0xFFU);
        d[3] = (uint8_t)((tss_b >> 8) & 0xFFU);
        d[4] = (uint8_t)((tss_b >> 16) & 0xFFU);
        d[5] = 0x89U; /* 64-bit TSS (available), P=1 */
        d[6] = (uint8_t)((lim >> 16) & 0x0FU);
        d[7] = (uint8_t)((tss_b >> 24) & 0xFFU);
        *(uint32_t*)(void*)(d + 8) = (uint32_t)(tss_b >> 32);
        *(uint32_t*)(void*)(d + 12) = 0U;
    }

    struct GDT_ptr gp;
    gp.limit = (uint16_t)(6U * 8U - 1U);
    gp.base = (uint64_t)(uintptr_t)gdt6;
    __asm__ volatile("lgdt %0" : : "m"(gp) : "memory");
    /* ltr is r/m16. gas turns `ltr %ax`/`ltrw %ax` into 0f 00 d8 (ltr %eax); in long mode use 0x66 prefix. */
    __asm__ volatile(
        "movw %0, %%ax\n\t"
        ".byte 0x66, 0x0f, 0x00, 0xd8"
        : : "i"((int)GDT_TSS_SEL) : "ax", "memory");
}

void idt_init(void)
{
    tss_ist_lgdt_ltr();

    uint64_t def = (uint64_t)(uintptr_t)default_cpu_exception;
    for (uint8_t n = 0; n < 32U; n++) {
        idt_set_gate(n, def, INTERRUPT_GATE, 0U);
    }
    /* #DF / #PF / #GP: diagnosable dumps on COM1; IST for #DF/#PF/#GP. */
    idt_set_gate(0x08, (uint64_t)(uintptr_t)exc_double_fault, INTERRUPT_GATE, 2U);
    idt_set_gate(0x0d, (uint64_t)(uintptr_t)exc_general_protection, INTERRUPT_GATE, 1U);
    idt_set_gate(0x0e, (uint64_t)(uintptr_t)exc_page_fault, INTERRUPT_GATE, 1U);

    /* IRQ0 timer / IRQ1 keyboard via central irq table (not hard-coded gates alone). */
    irq_init();
    irq_register(0, timer_interrupt_handler);
    irq_register(1, keyboard_handler_main);

    extern void syscall_handler_asm(void);
    uint64_t syscall_address = (uint64_t)(uintptr_t)syscall_handler_asm;
    idt_set_gate(0x80, syscall_address, 0xEEU, 0U);

    /*     Ports
    *    PIC1    PIC2
    *Command 0x20    0xA0
    *Data     0x21    0xA1
    */

    /* ICW1 - begin initialization */
    write_port(0x20 , 0x11);
    write_port(0xA0 , 0x11);

    /* ICW2 - remap offset address of IDT */
    /*
    * In x86 protected mode, we have to remap the PICs beyond 0x20 because
    * Intel has designated the first 32 interrupts as "reserved" for CPU exceptions
    */
    write_port(0x21 , 0x20);
    write_port(0xA1 , 0x28);

    /* ICW3 - setup cascading */
    write_port(0x21 , 0x00);
    write_port(0xA1 , 0x00);

    /* ICW4 - environment info */
    write_port(0x21 , 0x01);
    write_port(0xA1 , 0x01);
    /* Initialization finished */

    /* mask interrupts */
    write_port(0x21 , 0xff);
    write_port(0xA1 , 0xff);

    /* 64-bit: lidt must see a contiguous 2+8 byte block; avoid RDI/ABI issues by using "m". */
    {
        struct {
            uint16_t limit;
            uint64_t base;
        } __attribute__((packed)) idt_desc = {
            (uint16_t)(sizeof(struct IDT_entry) * IDT_SIZE - 1U),
            (uint64_t)(uintptr_t)IDT,
        };
        __asm__ volatile("lidt %0" : : "m"(idt_desc) : "memory");
    }
    /* Leave interrupts off until init_boot_screen finishes; UEFI laptops can mis-deliver IRQs here. */
}

