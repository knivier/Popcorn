; src/core/context_switch.asm
; New-task entry (Popcorn kernel)
;
; Tasks are switched by interrupt frames (see scheduler.c and idt.asm): a task
; that is not running is one saved *PopFrame on its own kernel stack. A new
; task's first frame (setup_task_context in scheduler.c) iretq's here with
; IF=0, r12 = task function, r13 = task data and RSP = stack top - 8.

bits 64
section .note.GNU-stack noalloc noexec nowrite progbits
section .text
global task_entry_trampoline
extern scheduler_task_entry

; scheduler_task_entry() enables interrupts as appropriate, runs the task
; function and exits the task; it never returns.
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
