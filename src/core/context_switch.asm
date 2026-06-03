; src/core/context_switch.asm
; x86-64 context switching (Popcorn kernel)
;
; RDI = CPUContext* (System V). context_save: save caller; context_restore: iretq
; to a stack with {RIP,CS,RFLAGS} for ring-0. C stacks that only have a return
; address are repaired by writing that frame 24 bytes below the saved RSP.

section .bss
align 8
context_save_ptr: resq 1
context_save_r10_tmp: resq 1 ; popped r10 before reloading context ptr into r10
context_restore_iret_rsp: resq 1

section .text
global context_save
global context_restore
global context_switch_to_task
extern scheduler_get_current_task

; void context_save(CPUContext* c)
context_save:
    mov [rel context_save_ptr], rdi
    mov r11, rdi
    mov [r11 + 112], rax

    cli
    mov rbx, rsp
    mov r11, [rel context_save_ptr]
    mov [r11 + 128], rbx

    push rbp
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rbp

    ; r10 = restored callee r10. Next line would reload context* into r10 — that
    ; destroyed the true r10 and [r10+40] was written with the pointer (bogus iretq RIP).
    mov [rel context_save_r10_tmp], r10
    mov r10, [rel context_save_ptr]
    mov [r10 + 0], r15
    mov [r10 + 8], r14
    mov [r10 + 16], r13
    mov [r10 + 24], r12
    mov [r10 + 32], r11
    mov r11, [rel context_save_r10_tmp]
    mov [r10 + 40], r11
    mov [r10 + 48], r9
    mov [r10 + 56], r8
    mov [r10 + 64], rdi
    mov [r10 + 72], rsi
    mov [r10 + 80], rbp
    mov [r10 + 88], rdx
    mov [r10 + 96], rcx
    mov [r10 + 104], rbx

    mov rbx, [rsp]
    mov [r10 + 120], rbx

    pushfq
    pop rbx
    mov [r10 + 136], rbx

    mov bx, cs
    mov [r10 + 144], bx
    mov bx, ss
    mov [r10 + 152], bx
    mov bx, ds
    mov [r10 + 160], bx
    mov bx, es
    mov [r10 + 168], bx
    mov bx, fs
    mov [r10 + 176], bx
    mov bx, gs
    mov [r10 + 184], bx

    mov qword [r10 + 192], 0x1
    ret

; void context_restore(CPUContext* c)
context_restore:
    mov r13, rdi
    cli

    mov bx, [r13 + 160]
    mov ds, bx
    mov bx, [r13 + 168]
    mov es, bx
    mov bx, [r13 + 176]
    mov fs, bx
    mov bx, [r13 + 184]
    mov gs, bx

    ; Ring-0 iretq: 3 qwords at RSP = {RIP,CS,RFLAGS}. C stacks may lack CS/RFLAGS
    ; at RSP+8/16, and [RSP+8]==0x8 can be a false positive. Always build the frame
    ; at (saved RSP) - 24 from fields in the context struct.
    mov r8, [r13 + 128]
    sub r8, 24
    mov r9, [r13 + 120]
    mov [r8], r9
    mov qword [r8 + 8], 0x8
    mov r9, [r13 + 136]
    mov [r8 + 16], r9
    mov qword [rel context_restore_iret_rsp], r8

.gpr:
    mov r14, r13
    mov r15, [r14 + 0]
    mov r12, [r14 + 24]
    mov r11, [r14 + 32]
    mov r10, [r14 + 40]
    mov r9, [r14 + 48]
    mov r8, [r14 + 56]
    mov rdi, [r14 + 64]
    mov rsi, [r14 + 72]
    mov rbp, [r14 + 80]
    mov rdx, [r14 + 88]
    mov rcx, [r14 + 96]
    mov rbx, [r14 + 104]
    mov rax, [r14 + 112]
    mov r13, [r14 + 16]
    mov r14, [r14 + 8]
    mov rsp, [rel context_restore_iret_rsp]
    iretq

; void context_switch_to_task(TaskStruct* t)
; TaskStruct.context offset must match src/includes/scheduler.h (see _Static_assert in scheduler.c).
context_switch_to_task:
    add rdi, 72
    jmp context_restore