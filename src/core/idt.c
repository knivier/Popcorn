#include "../includes/idt.h"
#include "../includes/irq.h"
#include "../includes/timer.h"
#include <stddef.h>
#include <stdint.h>

#include "../includes/popframe.h"

/* idt.asm: isr_stub_<n> for every vector; all of them build a PopFrame. */
extern const uint64_t isr_stub_table[IDT_SIZE];
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
#define GDT_KERNEL_CODE_SEL KERNEL_CODE_SEGMENT_OFFSET /* 0x08, IDT gate selector */
#define GDT_KERNEL_DATA_SEL KERNEL_DATA_SEGMENT_OFFSET /* 0x10 */
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
    /*
     * Reload every selector from the new table. Live selectors keep their old
     * hidden descriptor caches, but iretq (IRQ/exception/syscall return) re-reads
     * CS/SS from the GDT, so any selector not valid in gdt6 would #GP there. Both
     * entry paths already run on 0x08/0x10 from kernel.asm's boot GDT; this
     * keeps the invariant local to the GDT that is actually live. Far return
     * (lretq) is the only way to reload CS in long mode. Built with
     * -mno-red-zone, so the pushes cannot clobber locals.
     */
    __asm__ volatile(
        "lgdt %0\n\t"
        "pushq %1\n\t"
        "leaq 1f(%%rip), %%rax\n\t"
        "pushq %%rax\n\t"
        "lretq\n\t"
        "1:\n\t"
        "movw %2, %%ax\n\t"
        "movw %%ax, %%ds\n\t"
        "movw %%ax, %%es\n\t"
        "movw %%ax, %%fs\n\t"
        "movw %%ax, %%gs\n\t"
        "movw %%ax, %%ss\n\t"
        : : "m"(gp), "i"((int)GDT_KERNEL_CODE_SEL), "i"((int)GDT_KERNEL_DATA_SEL)
        : "rax", "memory");
    /* ltr is r/m16. gas turns `ltr %ax`/`ltrw %ax` into 0f 00 d8 (ltr %eax); in long mode use 0x66 prefix. */
    __asm__ volatile(
        "movw %0, %%ax\n\t"
        ".byte 0x66, 0x0f, 0x00, 0xd8"
        : : "i"((int)GDT_TSS_SEL) : "ax", "memory");
}

void idt_init(void)
{
    tss_ist_lgdt_ltr();

    /* Every vector gets a present gate to its own stub, so nothing can raise #NP:
     * exceptions name their real vector, and a stray interrupt on an unused vector
     * is counted by isr_dispatch instead of faulting. */
    for (unsigned n = 0; n < IDT_SIZE; n++) {
        idt_set_gate((uint8_t)n, isr_stub_table[n], INTERRUPT_GATE, 0U);
    }
    /* #DF / #PF / #GP on IST stacks so a blown kernel stack still produces a dump. */
    idt_set_gate(0x08, isr_stub_table[0x08], INTERRUPT_GATE, 2U);
    idt_set_gate(0x0d, isr_stub_table[0x0d], INTERRUPT_GATE, 1U);
    idt_set_gate(0x0e, isr_stub_table[0x0e], INTERRUPT_GATE, 1U);

    /* IRQ0 timer / IRQ1 keyboard via central irq table (not hard-coded gates alone). */
    irq_init();
    irq_register(0, timer_interrupt_handler);
    irq_register(1, keyboard_handler_main);

    /* int 0x80 is callable from Ring 3 (DPL 3); int 0x81 (yield) is kernel-only. */
    idt_set_gate(POP_VEC_SYSCALL, isr_stub_table[POP_VEC_SYSCALL], 0xEEU, 0U);
    idt_set_gate(POP_VEC_YIELD, isr_stub_table[POP_VEC_YIELD], INTERRUPT_GATE, 0U);

    /* PIC remap: vectors 0x20/0x28, ICW3 cascade IRQ2 (was wrongly 0x00/0x00). */
    pic_init_remap();
    /* Early boot: keep all IRQs masked until timer/kbd explicitly enable. */
    write_port(0x21, 0xff);
    write_port(0xA1, 0xff);

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

