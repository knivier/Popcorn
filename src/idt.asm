; src/idt.asm
global load_idt

load_idt:
    push ebp
    mov ebp, esp
    mov edx, [esp + 8]
    lidt [edx]
    pop ebp
    ret