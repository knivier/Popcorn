; src/core/context_switch.asm
; x86-64 context switching assembly functions for Popcorn kernel

; Global symbols
global context_save
global context_restore
global context_switch_to_task

; External symbols
extern scheduler_get_current_task

; Context save function
; Saves current CPU state to the provided context structure
; C signature: void context_save(CPUContext* context)
context_save:
    ; Get the context pointer from RDI (first argument)
    mov rax, rdi

    ; Disable interrupts during context save
    cli

    ; Save current stack pointer BEFORE pushing registers
    mov rbx, rsp
    mov [rax + 128], rbx  ; Save RSP

    ; Save general purpose registers (in reverse order to match push order)
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

    ; Restore the registers we just pushed (except RSP which we saved)
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

    ; Save the remaining registers
    mov [rax + 0], r15
    mov [rax + 8], r14
    mov [rax + 16], r13
    mov [rax + 24], r12
    mov [rax + 32], r11
    mov [rax + 40], r10
    mov [rax + 48], r9
    mov [rax + 56], r8
    mov [rax + 64], rdi
    mov [rax + 72], rsi
    mov [rax + 80], rbp
    mov [rax + 88], rdx
    mov [rax + 96], rcx
    mov [rax + 104], rbx
    mov [rax + 112], rax

    ; Save control registers
    ; The return address is at [rsp] after all our pushes
    mov rbx, [rsp]  ; Get return address from stack
    mov [rax + 120], rbx  ; Save RIP

    ; Save RFLAGS
    pushfq
    pop rbx
    mov [rax + 136], rbx  ; Save RFLAGS

    ; Save segment registers
    mov bx, cs
    mov [rax + 144], bx
    mov bx, ss
    mov [rax + 152], bx
    mov bx, ds
    mov [rax + 160], bx
    mov bx, es
    mov [rax + 168], bx
    mov bx, fs
    mov [rax + 176], bx
    mov bx, gs
    mov [rax + 184], bx

    ; Save FPU/SSE state (simplified - just mark as present)
    mov qword [rax + 192], 0x1  ; Mark FPU state as present

    ; Re-enable interrupts
    sti

    ret

; Context restore function
; Restores CPU state from the provided context structure
; C signature: void context_restore(CPUContext* context)
context_restore:
    ; Get the context pointer from RDI (first argument)
    mov rax, rdi

    ; Disable interrupts during context restore
    cli

    ; Restore segment registers first (data segments)
    mov bx, [rax + 160]  ; DS
    mov ds, bx
    mov bx, [rax + 168]  ; ES
    mov es, bx
    mov bx, [rax + 176]  ; FS
    mov fs, bx
    mov bx, [rax + 184]  ; GS
    mov gs, bx

    ; Restore general purpose registers first
    mov r15, [rax + 0]
    mov r14, [rax + 8]
    mov r13, [rax + 16]
    mov r12, [rax + 24]
    mov r11, [rax + 32]
    mov r10, [rax + 40]
    mov r9, [rax + 48]
    mov r8, [rax + 56]
    mov rdi, [rax + 64]
    mov rsi, [rax + 72]
    mov rbp, [rax + 80]
    mov rdx, [rax + 88]
    mov rcx, [rax + 96]
    mov rbx, [rax + 104]

    ; Restore RAX
    mov rax, [rax + 112]

    ; Switch to the task's stack (which has pre-filled iretq frame)
    mov rsp, [rax + 128]  ; RSP points to iretq frame

    ; Execute iretq to switch to the task
    iretq

; Context switch to specific task
; Switches to the provided task, saving current context
; C signature: void context_switch_to_task(TaskStruct* task)
context_switch_to_task:
    ; Get the new task pointer from RDI (first argument)
    mov rax, rdi

    ; Restore new task's context (offset 88 is context field in TaskStruct)
    mov rdi, rax
    add rdi, 88  ; Offset to context field in TaskStruct
    
    ; Jump directly to context_restore to avoid call/return issues
    jmp context_restore
