; src/core/context_switch.asm
; Kernel-thread context switch (Popcorn kernel)
;
; Every task runs on its own kernel stack. switch_to() pushes the SysV
; callee-saved registers (rbx, rbp, r12-r15) on the outgoing stack, stores RSP
; into *old_rsp, loads new_rsp, pops the incoming task's callee-saved registers
; and `ret`s to wherever that task last called switch_to(). Caller-saved
; registers (and XMM, which SysV treats as caller-saved) need no saving: the C
; caller already assumes the call clobbers them. A task preempted by an IRQ has
; its full GPR + FPU state saved by the IRQ stub (idt.asm) on its own stack.
;
; RFLAGS is not switched. The scheduler calls switch_to() with IF=0, and each
; task restores its own saved flags when its scheduler call returns.
;
; A new task's stack is prepared by setup_task_context() (scheduler.c) so that
; the first switch_to() into it pops zeros / entry arguments and `ret`s into
; task_entry_trampoline:
;   [top-8]  0                       (fake return address, ends backtraces)
;   [top-16] task_entry_trampoline   (switch_to's ret target)
;   [top-24] rbp = 0
;   [top-32] rbx = 0
;   [top-40] r12 = entry function
;   [top-48] r13 = task data
;   [top-56] r14 = 0
;   [top-64] r15 = 0                 <- saved RSP

bits 64
section .note.GNU-stack noalloc noexec nowrite progbits
section .text
global switch_to
global task_entry_trampoline
extern scheduler_task_entry

; void switch_to(uint64_t* old_rsp, uint64_t new_rsp)
switch_to:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov [rdi], rsp
    mov rsp, rsi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

; First code a new task runs (IF=0, still inside the scheduler's critical
; section). scheduler_task_entry() enables interrupts as appropriate, runs the
; task function and exits the task; it never returns.
task_entry_trampoline:
    xor ebp, ebp
    mov rdi, r12            ; void (*fn)(void)
    mov rsi, r13            ; void* data
    and rsp, -16            ; RSP % 16 == 0 at the call (SysV)
    call scheduler_task_entry
.hang:
    cli
    hlt
    jmp .hang
