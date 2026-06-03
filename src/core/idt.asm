; load_idt is in kernel.asm. Kept for link order; avoid duplicate globals.
bits 64
section .note.GNU-stack
section .text
global popcorn_idt_stub
popcorn_idt_stub:
  ret
