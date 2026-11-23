/* src/core/isr.c - the one C entry point for every interrupt vector.
 *
 * idt.asm builds a *PopFrame for every vector and calls isr_dispatch(); whatever
 * frame this returns is what the CPU resumes. Returning another task's saved frame
 * is a task switch (scheduler.c), so the scheduler hooks sit at the exits below. */
#include "../includes/popframe.h"
#include "../includes/exception.h"
#include "../includes/irq.h"
#include "../includes/lapic.h"
#include "../includes/scheduler.h"
#include "../includes/syscall.h"
#include "../includes/boot_fb.h"
#include <stdint.h>

/* Rust vector table (drivers/grant/irq.rs): counts the vector and marks it pending
 * for its owner. Returns 1 if a kernel or device owner holds the vector. */
extern uint32_t rust_vector_dispatch(uint64_t vector);

static uint32_t g_unexpected_vectors;
static uint32_t g_lapic_spurious;

/* int 0x80: syscall_context_t view of the frame; the result goes back in RAX. */
static PopFrame* isr_syscall(PopFrame* f) {
    syscall_context_t ctx = {
        .rax = f->rax, .rdi = f->rdi, .rsi = f->rsi, .rdx = f->rdx,
        .rcx = f->rcx, .r8 = f->r8, .r9 = f->r9,
        .rsp = f->rsp, .rip = f->rip, .rflags = f->rflags, .cs = f->cs, .ss = f->ss,
    };
    /* The gate clears IF. Give the handler the caller's IF so blocking syscalls can be
     * preempted; poll-mode callers keep IF=0. */
    if (f->rflags & 0x200u) {
        __asm__ volatile("sti" ::: "memory");
    }
    int64_t result = syscall_dispatch(&ctx);
    __asm__ volatile("cli" ::: "memory");
    f->rax = (uint64_t)result;
    return f;
}

/* A misaligned frame means a stub pushed the wrong number of words: every field C
 * reads (and the FXSAVE area) would be shifted. Stop loudly instead. */
static void __attribute__((noreturn)) isr_bad_frame(const PopFrame* f) {
    static const char msg[] = "\r\n*** CPU EXCEPTION #FRAME: misaligned PopFrame, halting ***\r\n";
    (void)f;
    for (const char* p = msg; *p; p++) {
        boot_serial_putc(*p);
    }
    for (;;) {
        __asm__ volatile("cli; hlt");
    }
}

PopFrame* isr_dispatch(PopFrame* f) {
    if (((uintptr_t)f & 15u) != 0 || f->vector > 0xFFu) {
        isr_bad_frame(f);
    }
    const uint64_t v = f->vector;

    if (v < 0x20u) {
        return exception_handle(f);
    }
    if (v < 0x30u) {
        /* PIC IRQ 0-15: handler (EOI policy lives there), then maybe preempt. */
        (void)rust_vector_dispatch(v); /* count only (the `irq` listing) */
        irq_dispatch((uint8_t)(v - 0x20u));
        return scheduler_on_irq_exit(f);
    }
    if (v == POP_VEC_SYSCALL) {
        return isr_syscall(f);
    }
    if (v == POP_VEC_YIELD) {
        return scheduler_on_yield(f);
    }
    if (v == POP_VEC_SPURIOUS) {
        g_lapic_spurious++;  /* LAPIC spurious: no EOI by definition */
        return f;
    }
    /* LAPIC-delivered vectors (self-test, MSI-X from M2): the Rust table counts it and
     * flags it pending for its owner, which consumes it from its own pump. A vector
     * nobody owns is counted as unexpected. Either way it was LAPIC-delivered: EOI. */
    if (!rust_vector_dispatch(v)) {
        g_unexpected_vectors++;
    }
    lapic_eoi();
    return f;
}

uint32_t isr_unexpected_vector_count(void) {
    return g_unexpected_vectors;
}

uint32_t isr_lapic_spurious_count(void) {
    return g_lapic_spurious;
}
