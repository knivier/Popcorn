;; kernel.asm - boot at 0x100000, kernel VA = 0xFFFF800000000000 + (LMA - 1M)

section .note.GNU-stack noalloc noexec nowrite progbits

section .multiboot
  align 8
multiboot2_header_start:
  dd 0xE85250D6
  dd 0
  dd multiboot2_header_end - multiboot2_header_start
  dd -(0xE85250D6 + 0 + (multiboot2_header_end - multiboot2_header_start))
  align 8
  dw 0
  dw 0
  dd 8
multiboot2_header_end:

section .text.boot
bits 32
global start
extern kmain

start:
  cli
  mov DWORD [multiboot2_info_ptr], ebx
  mov DWORD [multiboot2_info_ptr + 4], 0

  ; Page tables: L4 + 3 levels identity + l3 for high half (share L2)
  mov edi, page_table_l4
  mov ecx, 4096
  xor eax, eax
  rep stosd

  mov edi, page_table_l4
  mov DWORD [edi], page_table_l3 + 0x003

  mov edi, page_table_l3
  mov DWORD [edi], page_table_l2 + 0x003

  mov edi, page_table_l2
  mov ecx, 512
  xor ebx, ebx
.p2_2m_loop:
  lea eax, [edi+ebx*8]
  mov edx, ebx
  shl edx, 21
  or  edx, 0x083
  mov [eax], edx
  mov DWORD [eax+4], 0
  inc ebx
  dec ecx
  jne .p2_2m_loop

  ; PML4[256] -> l3_h -> (same) L2: VA 0xFFFF800000000000 maps to phys 0..1G
  mov eax, page_table_l3_high
  or  eax, 0x3
  mov [page_table_l4 + 8*256], eax
  mov DWORD [page_table_l4 + 8*256 + 4], 0

  mov eax, page_table_l2
  or  eax, 0x3
  mov [page_table_l3_high], eax
  mov DWORD [page_table_l3_high+4], 0

  mov eax, cr4
  or  eax, 1 << 5
  mov cr4, eax

  mov eax, page_table_l4
  mov cr3, eax

  mov ecx, 0xC0000080
  rdmsr
  or  eax, 1 << 8
  wrmsr

  mov eax, cr0
  or  eax, 1 << 31
  mov cr0, eax

  lgdt [gdt64_pointer]
  jmp 0x08:start64_tramp

bits 64
start64_tramp:
  mov ax, 0x10
  mov ds, ax
  mov es, ax
  mov fs, ax
  mov gs, ax
  mov ss, ax

  mov rax, cr0
  and rax, ~0x4
  or  rax, 0x2
  mov cr0, rax
  mov rax, cr4
  or  rax, (1 << 9) | (1 << 10)
  mov cr4, rax

  mov rsp, stack_top

  mov rax, kmain
  call rax

  cli
.hang:
  hlt
  jmp .hang

section .rodata.boot
align 8
gdt64:
  dq 0
  dq 0x00209A0000000000
  dq 0x0000920000000000

gdt64_pointer:
  dw $ - gdt64 - 1
  dq gdt64

section .bss.boot
; Multiboot pointer first (low); page tables and stack are zeroed/used by rep stosd
; and must not be aliased with stack_top (stack grew down into page_table_* before).
align 8
global multiboot2_info_ptr
multiboot2_info_ptr:
  resq 1
align 4096
page_table_l4:
  resb 4096
page_table_l3:
  resb 4096
page_table_l2:
  resb 4096
page_table_l3_high:
  resb 4096
stack_bottom:
  resb 131072
global stack_top
stack_top:

; --- high-half linked code (handlers, I/O) ---
section .text
global keyboard_handler
global timer_handler
global syscall_handler_asm
global read_port
global write_port
global load_idt
global cpuid_get_vendor
global cpuid_get_features
global cpuid_extended_brand
global rdtsc
global default_cpu_exception

extern keyboard_handler_main
extern timer_interrupt_handler
extern syscall_dispatch

; CPU exception vectors 0x00–0x1F: halt if unhandled. Presents a valid gate
; so a fault (e.g. #PF) does not re-fault on an empty IDT entry.
default_cpu_exception:
  cli
.hang:
  hlt
  jmp .hang

read_port:
  mov rdx, rdi
  xor rax, rax
  in al, dx
  ret

write_port:
  mov rdx, rdi
  mov rax, rsi
  out dx, al
  ret

load_idt:
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

timer_handler:
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
  call timer_interrupt_handler
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

syscall_handler_asm:
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
  mov rdi, rsp
  add rdi, 120
  call syscall_dispatch
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

cpuid_get_vendor:
  push rbx
  xor eax, eax
  cpuid
  mov [rdi], ebx
  mov [rdi+4], edx
  mov [rdi+8], ecx
  pop rbx
  ret

cpuid_get_features:
  push rbx
  mov eax, 1
  cpuid
  mov [rdi], eax
  mov [rdi+4], ebx
  mov [rdi+8], ecx
  mov [rdi+12], edx
  pop rbx
  ret

cpuid_extended_brand:
  push rbx
  mov eax, edi
  xor ecx, ecx
  cpuid
  mov [rsi], eax
  mov [rsi+4], ebx
  mov [rsi+8], ecx
  mov [rsi+12], edx
  pop rbx
  ret

rdtsc:
  rdtsc
  shl rdx, 32
  or rax, rdx
  ret

; context_switch lives in context_switch.asm
