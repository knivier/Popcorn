;;kernel.asm

;nasm directive - 32 bit
bits 32
section .note.GNU-stack noalloc noexec nowrite progbits
section .text
        align 4
        dd 0x1BADB002            ;magic
        dd 0x00                  ;flags
        dd - (0x1BADB002 + 0x00) ;checksum. m+f+c should be zero

global start
extern kmain	        ;kmain in kernel.c

start:
  cli 			
  mov esp, stack_space	;set stack pointer
  call kmain

section .bss
resb 16000		;16KB for stack
stack_space: