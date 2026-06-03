;; kernel.asm - boot at 0x100000, kernel VA = 0xFFFF800000000000 + (LMA - 1M)

%define NUM_L2_IDENTITY 64        ; 64 x 512 x 2 MiB = 64 GiB identity map
%define MBI_STAGING_SIZE 65536

section .note.GNU-stack noalloc noexec nowrite progbits

section .multiboot
  align 8
multiboot2_header_start:
  dd 0xE85250D6
  dd 0
  dd multiboot2_header_end - multiboot2_header_start
  dd -(0xE85250D6 + 0 + (multiboot2_header_end - multiboot2_header_start))
  ; Framebuffer request tag: ask GRUB for a linear RGB framebuffer.
  ; Required on UEFI-only machines (no CSM) where legacy VGA text at 0xB8000
  ; is not available. width=0, height=0 => GRUB picks a mode; depth=32.
  align 8
  dw 5      ; MULTIBOOT_HEADER_TAG_FRAMEBUFFER
  dw 1      ; flags (1 = optional)
  dd 20     ; size
  dd 0      ; width  (no preference)
  dd 0      ; height (no preference)
  dd 0      ; depth  (no preference)
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

  ; Do not dereference EBX here: on some Ventoy/UEFI paths a bad pointer here
  ; would fault before IDT is installed and hang at GRUB "Booting kernel...".
  mov DWORD [multiboot2_info_ptr], ebx
  mov DWORD [multiboot2_info_ptr + 4], 0

  ; Zero page tables: l4 + l3 + NUM_L2_IDENTITY x l2 + l3_high
  mov edi, page_table_l4
  mov ecx, (2 + NUM_L2_IDENTITY + 1) * 1024
  xor eax, eax
  rep stosd

  ; PML4[0] -> l3 (low-half identity)
  mov DWORD [page_table_l4], page_table_l3 + 0x003
  mov DWORD [page_table_l4 + 4], 0

  ; PML4[256] -> l3_high (high-half kernel at 0xFFFF800000000000)
  mov eax, page_table_l3_high
  or  eax, 0x003
  mov [page_table_l4 + 8*256], eax
  mov DWORD [page_table_l4 + 8*256 + 4], 0

  ; Identity-map first 64 GiB (framebuffer / firmware structures may sit above 4 GiB).
  mov edi, page_table_l2
  mov ecx, NUM_L2_IDENTITY * 512
  xor ebx, ebx
.p2_2m_loop:
  lea eax, [edi+ebx*8]
  ; Build full 64-bit 2 MiB page physical base: (ebx << 21).
  ; low32  = ebx << 21
  ; high32 = ebx >> 11
  mov edx, ebx
  shl edx, 21
  mov esi, ebx
  shr esi, 11
  or  edx, 0x083
  mov [eax], edx
  mov [eax+4], esi
  inc ebx
  dec ecx
  jne .p2_2m_loop

  xor ecx, ecx
.l3_link_loop:
  mov eax, ecx
  shl eax, 12
  add eax, page_table_l2
  or  eax, 0x003
  mov edx, ecx
  shl edx, 3
  mov [page_table_l3 + edx], eax
  mov DWORD [page_table_l3 + edx + 4], 0
  mov [page_table_l3_high + edx], eax
  mov DWORD [page_table_l3_high + edx + 4], 0
  inc ecx
  cmp ecx, NUM_L2_IDENTITY
  jl .l3_link_loop

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

;; UEFI firmware enters in 64-bit long mode. Do not jump to 32-bit `start`.
%define POPCORN_UEFI_MAGIC_CONST 0x504F50434F524E42
%define POPCORN_HANDOFF_PHYS     0x102008
%define POPCORN_UEFI_MBI_PHYS    0x380000

bits 64
global uefi_start
extern kmain

; If GOP handoff is valid, paint a cyan strip at the top (proves kernel CR3 + FB work).
uefi_fb_alive_marker:
  mov rax, [POPCORN_HANDOFF_PHYS]
  cmp rax, POPCORN_UEFI_MAGIC_CONST
  jne .done
  mov rbx, [POPCORN_HANDOFF_PHYS + 8]
  test rbx, rbx
  jz .done
  mov ecx, [POPCORN_HANDOFF_PHYS + 16]   ; pitch
  mov edx, [POPCORN_HANDOFF_PHYS + 20]   ; width
  mov esi, [POPCORN_HANDOFF_PHYS + 24]   ; height
  test ecx, ecx
  jz .done
  test edx, edx
  jz .done
  cmp esi, 8
  jl .done
  xor edi, edi
.row:
  cmp edi, 8
  jge .done
  mov rax, rbx
  mov r9, rdi
  imul r9, rcx
  add rax, r9
  xor r8d, r8d
.col:
  cmp r8d, edx
  jge .next_row
  mov byte [rax + r8*4 + 0], 0xFF     ; B
  mov byte [rax + r8*4 + 1], 0xFF     ; G
  mov byte [rax + r8*4 + 2], 0x00     ; R
  mov byte [rax + r8*4 + 3], 0
  inc r8d
  jmp .col
.next_row:
  inc edi
  jmp .row
.done:
  ret

uefi_start:
  cli

  ; Zero only page-table blobs (not the stack — rep at stack_bottom was slow and fragile).
  lea rdi, [rel page_table_l4]
  mov ecx, (2 + NUM_L2_IDENTITY + 1) * 512
  xor rax, rax
  rep stosq

  lea rax, [rel page_table_l3]
  or rax, 0x003
  lea rdi, [rel page_table_l4]
  mov [rdi], rax
  mov qword [rdi + 8], 0

  lea rax, [rel page_table_l3_high]
  or rax, 0x003
  mov [rdi + 8 * 256], rax
  mov qword [rdi + 8 * 256 + 8], 0

  xor ecx, ecx
.uefi_l3_1g_loop:
  lea rdi, [rel page_table_l3]
  mov rax, rcx
  shl rax, 30
  or  rax, 0x083
  mov [rdi + rcx*8], rax
  inc ecx
  cmp ecx, 512
  jl .uefi_l3_1g_loop

  xor ecx, ecx
.uefi_l3h_1g_loop:
  lea rdi, [rel page_table_l3_high]
  mov rax, rcx
  shl rax, 30
  or  rax, 0x083
  mov [rdi + rcx*8], rax
  inc ecx
  cmp ecx, NUM_L2_IDENTITY
  jl .uefi_l3h_1g_loop

  mov rax, cr4
  or eax, 1 << 5
  mov cr4, rax

  lea rax, [rel page_table_l4]
  mov cr3, rax

  mov rsp, stack_top

  mov rax, [POPCORN_HANDOFF_PHYS]
  cmp rax, POPCORN_UEFI_MAGIC_CONST
  jne .uefi_no_mbi
  mov qword [rel multiboot2_info_ptr], POPCORN_UEFI_MBI_PHYS
  mov qword [rel multiboot2_info_ptr + 8], 0
.uefi_no_mbi:

  ; Keep firmware GDT/segments — retfq + SS=0x10 breaks on many Lenovo/AMD UEFI boxes.
  mov rax, kmain
  call rax

  cli
.uefi_hang:
  hlt
  jmp .uefi_hang

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

  ; Optional COM1 marker (do not block boot if UART is absent/not-ready).
  mov dx, 0x3FD
  in al, dx
  test al, 0x20
  jz .skip_serial_marker
  mov dx, 0x3F8
  mov al, 'K'
  out dx, al
.skip_serial_marker:

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
; UEFI loader writes GOP handoff here (physical address in boot segment).
align 8
global popcorn_uefi_handoff
popcorn_uefi_handoff:
  resb 96
align 4096
global page_table_l2
global page_table_l3
page_table_l4:
  resb 4096
page_table_l3:
  resb 4096
page_table_l2:
  resb NUM_L2_IDENTITY * 4096
page_table_l3_high:
  resb 4096
stack_bottom:
  resb 131072
global stack_top
stack_top:
align 4096
mbi_staging:
  resb MBI_STAGING_SIZE

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
  sub rsp, 8
  call keyboard_handler_main
  add rsp, 8
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
  sub rsp, 8
  call timer_interrupt_handler
  add rsp, 8
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
