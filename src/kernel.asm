;;kernel.asm - 64-bit version

;nasm directive - 64 bit
bits 32
section .note.GNU-stack noalloc noexec nowrite progbits

section .multiboot
  align 8
multiboot2_header_start:
  dd 0xE85250D6                ; Multiboot2 magic
  dd 0                         ; Architecture: 0 = i386
  dd multiboot2_header_end - multiboot2_header_start  ; Header length
  dd -(0xE85250D6 + 0 + (multiboot2_header_end - multiboot2_header_start))  ; Checksum
  
  ; End tag
  align 8
  dw 0    ; type
  dw 0    ; flags
  dd 8    ; size
multiboot2_header_end:

section .text
global start
extern kmain

start:
  cli                      ;block interrupts
  
  ; Set up page tables for long mode
  ; Clear page table memory
  mov edi, page_table_l4
  mov ecx, 4096
  xor eax, eax
  rep stosd
  
  ; Set up page tables: Identity map first 2MB
  mov edi, page_table_l4
  mov DWORD [edi], page_table_l3 + 0x003  ; present + writable
  
  mov edi, page_table_l3
  mov DWORD [edi], page_table_l2 + 0x003  ; present + writable
  
  mov edi, page_table_l2
  mov DWORD [edi], 0x000000 + 0x083       ; present + writable + huge (2MB page)
  
  ; Enable PAE
  mov eax, cr4
  or eax, 1 << 5
  mov cr4, eax
  
  ; Load page table
  mov eax, page_table_l4
  mov cr3, eax
  
  ; Enable long mode
  mov ecx, 0xC0000080  ; EFER MSR
  rdmsr
  or eax, 1 << 8       ; Set LM bit
  wrmsr
  
  ; Enable paging
  mov eax, cr0
  or eax, 1 << 31
  mov cr0, eax
  
  ; Load 64-bit GDT
  lgdt [gdt64_pointer]
  
  ; Far jump to 64-bit code
  jmp 0x08:start64

bits 64
start64:
  ; Set up segment registers
  mov ax, 0x10
  mov ds, ax
  mov es, ax
  mov fs, ax
  mov gs, ax
  mov ss, ax
  
  ; Set up stack
  mov rsp, stack_top
  
  ; Call kernel main
  call kmain
  
  ; Halt if kernel returns
  cli
.hang:
  hlt
  jmp .hang

global keyboard_handler
global read_port
global write_port
global load_idt

extern keyboard_handler_main

read_port:
  ; Parameter in rdi (first arg in x64 calling convention)
  mov rdx, rdi
  xor rax, rax
  in al, dx
  ret

write_port:
  ; First parameter (port) in rdi, second (data) in rsi
  mov rdx, rdi
  mov rax, rsi
  out dx, al
  ret

load_idt:
  ; Parameter in rdi
  lidt [rdi]
  sti
  ret

keyboard_handler:
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
  
  call keyboard_handler_main
  
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
  
  iretq

; GDT for 64-bit mode
section .rodata
align 8
gdt64:
  dq 0                     ; Null descriptor
  dq 0x00209A0000000000    ; 64-bit code segment (executable, readable)
  dq 0x0000920000000000    ; 64-bit data segment (writable)

gdt64_pointer:
  dw $ - gdt64 - 1         ; GDT limit
  dq gdt64                 ; GDT base

; Page tables (must be page-aligned)
section .bss
align 4096
page_table_l4:
  resb 4096
page_table_l3:
  resb 4096
page_table_l2:
  resb 4096

; Stack
align 16
stack_bottom:
  resb 16384               ; 16KB stack
stack_top: