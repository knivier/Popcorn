; src/idt.asm
section .note.GNU-stack
global load_idt

load_idt:
    push ebp
    mov ebp, esp
    mov edx, [esp + 8]
    lidt [edx]
    pop ebp
    ret