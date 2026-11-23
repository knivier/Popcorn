; src/core/idt.asm - interrupt entry stubs (Popcorn kernel)
;
; Every stub here saves all 15 GPRs and calls C with RSP 16-byte aligned
; (SysV: RSP % 16 == 0 at the `call`). Alignment is forced with
; `mov rbp, rsp / and rsp, -16` so it holds no matter how many words the CPU
; pushed; rbp is callee-saved, so it survives the call and restores RSP.
;
; IRQ and syscall stubs also fxsave/fxrstor the interrupted SSE/x87 state on
; the interrupted task's own stack. C is compiled with SSE allowed, and an IRQ
; can preempt a task mid-sequence and switch to another task, so XMM state must
; be preserved across the handler (and across any task switch inside it).
;
; Saved-GPR block layout (push rax first, r15 last), offsets from the block:
;   r15=0 r14=8 r13=16 r12=24 r11=32 r10=40 r9=48 r8=56
;   rbp=64 rdi=72 rsi=80 rdx=88 rcx=96 rbx=104 rax=112
; Exceptions: [block+120]=vector, [block+128]=error code, then the CPU frame
; RIP/CS/RFLAGS/RSP/SS. This matches exc_frame_t in exception.c.
; IRQs: [block+120]=ISA IRQ number, then the CPU frame.

bits 64
section .note.GNU-stack noalloc noexec nowrite progbits
section .text

global default_cpu_exception
global exc_double_fault
global exc_general_protection
global exc_page_fault
global exc_stub_table
global irq_stub_table
global keyboard_handler
global timer_handler
global syscall_handler_asm

extern irq_dispatch
extern syscall_dispatch
extern cpu_exception_panic

%define EXC_VECTOR_UNKNOWN 0xFFFF

%macro PUSH_GPRS 0
  push rax
  push rbx
  push rcx
  push rdx
  push rsi
  push rdi
  push rbp
  push r8
  push r9
  push r10
  push r11
  push r12
  push r13
  push r14
  push r15
%endmacro

%macro POP_GPRS 0
  pop r15
  pop r14
  pop r13
  pop r12
  pop r11
  pop r10
  pop r9
  pop r8
  pop rbp
  pop rdi
  pop rsi
  pop rdx
  pop rcx
  pop rbx
  pop rax
%endmacro

; ---- CPU exceptions 0x00-0x1F: dump vector / error / regs on COM1, halt ----

; Vector without a CPU error code: push a 0 placeholder so the frame is uniform.
%macro EXC_NOERR 1
exc_stub_%1:
  push qword 0
  push qword %1
  jmp cpu_exception_common
%endmacro

; Vector where the CPU already pushed an error code.
%macro EXC_ERR 1
exc_stub_%1:
  push qword %1
  jmp cpu_exception_common
%endmacro

EXC_NOERR 0
EXC_NOERR 1
EXC_NOERR 2
EXC_NOERR 3
EXC_NOERR 4
EXC_NOERR 5
EXC_NOERR 6
EXC_NOERR 7
exc_double_fault:         ; #DF (idt.c: IST2)
EXC_ERR   8
EXC_NOERR 9
EXC_ERR   10
EXC_ERR   11
EXC_ERR   12
exc_general_protection:   ; #GP (idt.c: IST1)
EXC_ERR   13
exc_page_fault:           ; #PF (idt.c: IST1)
EXC_ERR   14
EXC_NOERR 15
EXC_NOERR 16
EXC_ERR   17
EXC_NOERR 18
EXC_NOERR 19
EXC_NOERR 20
EXC_ERR   21
EXC_NOERR 22
EXC_NOERR 23
EXC_NOERR 24
EXC_NOERR 25
EXC_NOERR 26
EXC_NOERR 27
EXC_NOERR 28
EXC_ERR   29
EXC_ERR   30
EXC_NOERR 31

; Shared catch-all gate: the vector (and whether an error code was pushed) is
; unknown, so the dump flags it. Prefer installing exc_stub_table[n] per vector.
default_cpu_exception:
  push qword 0
  push qword EXC_VECTOR_UNKNOWN
  jmp cpu_exception_common

cpu_exception_common:
  cli
  PUSH_GPRS
  cld
  mov rdi, rsp            ; const exc_frame_t*
  mov rsi, cr2            ; CR2 (meaningful for #PF)
  and rsp, -16            ; RSP % 16 == 0 at the call (SysV)
  call cpu_exception_panic
.hang_exc:
  cli
  hlt
  jmp .hang_exc

; ---- ISA IRQs 0-15 (vectors 0x20-0x2F after PIC remap) ----

%macro IRQ_STUB 1
irq_stub_%1:
  push qword %1
  jmp irq_common
%endmacro

timer_handler:            ; IRQ0 (PIT)
IRQ_STUB 0
keyboard_handler:         ; IRQ1 (PS/2 keyboard)
IRQ_STUB 1
IRQ_STUB 2
IRQ_STUB 3
IRQ_STUB 4
IRQ_STUB 5
IRQ_STUB 6
IRQ_STUB 7
IRQ_STUB 8
IRQ_STUB 9
IRQ_STUB 10
IRQ_STUB 11
IRQ_STUB 12
IRQ_STUB 13
IRQ_STUB 14
IRQ_STUB 15

; irq_dispatch() may call the scheduler, which can switch to another task's
; stack and come back much later. Everything this stub needs is on the
; interrupted task's own stack, so the unwind below is correct whenever the
; task is resumed. EOI policy lives in C (the timer EOIs before scheduling).
irq_common:
  PUSH_GPRS
  cld
  mov rbp, rsp
  and rsp, -16
  sub rsp, 512
  fxsave64 [rsp]
  mov edi, [rbp + 120]    ; ISA IRQ number
  call irq_dispatch
  fxrstor64 [rsp]
  mov rsp, rbp
  POP_GPRS
  add rsp, 8              ; drop IRQ number
  iretq

; ---- int 0x80 ----
; Builds a syscall_context_t (syscall.h field order: rax rdi rsi rdx rcx r8 r9
; rsp rip rflags cs ss) below the saved GPRs, passes it in RDI, and returns
; syscall_dispatch's result to the caller in RAX.
%define SC_CTX 96         ; sizeof(syscall_context_t)
%define SC_REG(off) (SC_CTX + (off))
syscall_handler_asm:
  PUSH_GPRS
  cld
  sub rsp, SC_CTX
  mov rax, [rsp + SC_REG(112)]  ; caller rax (syscall number)
  mov [rsp + 0], rax
  mov [rsp + 8], rdi
  mov [rsp + 16], rsi
  mov [rsp + 24], rdx
  mov [rsp + 32], rcx
  mov [rsp + 40], r8
  mov [rsp + 48], r9
  mov rax, [rsp + SC_REG(144)]  ; interrupted RSP
  mov [rsp + 56], rax
  mov rax, [rsp + SC_REG(120)]  ; RIP
  mov [rsp + 64], rax
  mov rax, [rsp + SC_REG(136)]  ; RFLAGS
  mov [rsp + 72], rax
  mov rax, [rsp + SC_REG(128)]  ; CS
  mov [rsp + 80], rax
  mov rax, [rsp + SC_REG(152)]  ; SS
  mov [rsp + 88], rax
  mov rdi, rsp                  ; syscall_context_t*
  mov rbp, rsp
  and rsp, -16
  sub rsp, 512
  fxsave64 [rsp]
  ; The gate is an interrupt gate (IF=0). Give the handler the caller's IF so
  ; blocking syscalls can be preempted; poll-mode callers keep IF=0.
  test qword [rbp + SC_REG(136)], 0x200
  jz .sc_call
  sti
.sc_call:
  call syscall_dispatch
  cli
  fxrstor64 [rsp]
  mov rsp, rbp
  add rsp, SC_CTX
  mov [rsp + 112], rax          ; result -> saved rax slot
  POP_GPRS
  iretq

section .rodata
align 8
exc_stub_table:
%assign i 0
%rep 32
  dq exc_stub_ %+ i
%assign i i+1
%endrep

irq_stub_table:
%assign i 0
%rep 16
  dq irq_stub_ %+ i
%assign i i+1
%endrep
