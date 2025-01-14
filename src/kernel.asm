; src/kernel.asm
bits 32

section .multiboot
    align 4
    dd 0x1BADB002            ; magic
    dd 0x00                  ; flags
    dd - (0x1BADB002 + 0x00) ; checksum. m + f + c should be zero

section .text
global _start
extern kmain

_start:
    cli                  ; block interrupts
    mov esp, stack_space ; set stack pointer
    call kmain
    hlt                  ; halt the CPU

section .bss
resb 1048576      ; 1MB for stack
stack_space: