#ifndef POPFRAME_H
#define POPFRAME_H

#include <stddef.h>
#include <stdint.h>

/*
 * *PopFrame: the one interrupt frame every entry path builds (idt.asm isr_common):
 * CPU exceptions, IRQs, int 0x80 (syscalls) and int 0x81 (scheduler yield).
 *
 * Stack at the C call, low to high address:
 *   [ FXSAVE area, 512 bytes ]  <- POPFRAME_FPU(f); x87/SSE state of the interrupted code
 *   [ PopFrame              ]  <- f (16-byte aligned)
 *
 * The stub pushes `error` (0 when the CPU pushes none) and `vector`, then the 15
 * GPRs (rax first, so r15 ends lowest). rip..ss are what the CPU pushed; in long
 * mode rsp/ss are always present. A task that is switched out is exactly one
 * saved PopFrame on its own kernel stack: resuming it means handing that frame
 * back to isr_common, which restores the FPU area and GPRs and does iretq.
 */
typedef struct PopFrame {
    uint64_t r15, r14, r13, r12, r11, r10, r9, r8;
    uint64_t rbp, rdi, rsi, rdx, rcx, rbx, rax;
    uint64_t vector;
    uint64_t error;
    uint64_t rip, cs, rflags, rsp, ss;
} PopFrame;

#define POPFRAME_FPU_BYTES 512u
#define POPFRAME_FPU(f) ((uint8_t*)(f) - POPFRAME_FPU_BYTES)

_Static_assert(sizeof(PopFrame) == 176, "PopFrame size (idt.asm isr_common)");
_Static_assert(offsetof(PopFrame, rax) == 112, "PopFrame GPR block (idt.asm PUSH_GPRS)");
_Static_assert(offsetof(PopFrame, vector) == 120, "PopFrame vector slot");
_Static_assert(offsetof(PopFrame, error) == 128, "PopFrame error slot");
_Static_assert(offsetof(PopFrame, rip) == 136, "PopFrame CPU frame");

/* Software vectors (besides exceptions 0x00-0x1F and PIC IRQs 0x20-0x2F). */
#define POP_VEC_SYSCALL 0x80u  /* int 0x80, DPL 3 */
#define POP_VEC_YIELD   0x81u  /* int 0x81, DPL 0: voluntary task switch */
#define POP_VEC_SPURIOUS 0xFFu /* LAPIC spurious vector (no EOI) */

/* idt.asm -> C. Returns the frame to resume: f itself, or another task's saved frame. */
PopFrame* isr_dispatch(PopFrame* f);

#endif /* POPFRAME_H */
