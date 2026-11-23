; src/core/idt.asm - interrupt entry stubs (Popcorn kernel)
;
; One stub per vector (0-255), all funnelling into isr_common, which builds a
; *PopFrame (src/includes/popframe.h) and calls isr_dispatch(PopFrame*). The C
; side returns the frame to resume: the same one, or another task's saved frame
; (that is a task switch). Layout, low to high:
;
;   [FXSAVE 512][r15 r14 r13 r12 r11 r10 r9 r8 rbp rdi rsi rdx rcx rbx rax]
;   [vector][error][rip cs rflags rsp ss]   <- error = 0 when the CPU pushes none
;
; Alignment: in long mode the CPU aligns RSP to 16 before pushing its frame. With
; or without a CPU error code, frame + error + vector + 15 GPRs = 176 bytes, so
; the PopFrame and the FXSAVE area below it are 16-byte aligned, and RSP % 16 == 0
; at the call (SysV).
;
; The FXSAVE area holds the interrupted code's x87/SSE state (C is built with SSE,
; and a task switch can happen inside any handler), so each saved frame carries
; its own FPU state.

bits 64
section .note.GNU-stack noalloc noexec nowrite progbits
section .text

global isr_stub_table
extern isr_dispatch

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

; Vectors where the CPU pushes an error code: #DF #TS #NP #SS #GP #PF #AC #CP #VC #SX.
%macro ISR_STUB 1
isr_stub_%1:
%if %1 = 8 || (%1 >= 10 && %1 <= 14) || %1 = 17 || %1 = 21 || %1 = 29 || %1 = 30
%else
  push qword 0            ; uniform frame: placeholder error code
%endif
  push qword %1
  jmp isr_common
%endmacro

%assign v 0
%rep 256
ISR_STUB v
%assign v v+1
%endrep

isr_common:
  PUSH_GPRS
  cld
  sub rsp, 512
  fxsave64 [rsp]
  lea rdi, [rsp + 512]    ; PopFrame*
  call isr_dispatch       ; rax = PopFrame* to resume (maybe another task's)
  lea rsp, [rax - 512]
  fxrstor64 [rsp]
  add rsp, 512
  POP_GPRS
  add rsp, 16             ; drop vector + error
  iretq

section .rodata
align 8
isr_stub_table:
%assign v 0
%rep 256
  dq isr_stub_ %+ v
%assign v v+1
%endrep
