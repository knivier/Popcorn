/*
 * 64-bit kernel: physical load 1 MiB, virtual base 0xFFFF800000000000 (high half).
 * Boot code + early data live at 0x100000 LMA; C and most asm at VA = KERNEL_VMA_BASE + LMA
 * (see the note above .text).
 * GRUB: ELF entry = start, still at LMA 0x100000.
 * UEFI: BOOTX64.EFI jumps to the uefi_start symbol and writes its handoff at the
 * fixed address POPCORN_UEFI_HANDOFF_PHYS (asserted at the end of this file).
 */
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(start)

KERNEL_VMA_BASE = 0xFFFF800000000000;
BOOT_LMA = 0x100000;

/* Explicit PHDRS: (1) first segment must be R|E for .text.boot; R|W for .bss.boot.
   (2) A separate last PT_LOAD for .bss only can get p_offset=0x1000 and break GRUB.
     Put .data and .bss in the same :data PT_LOAD so one RW segment + correct file offset. */
PHDRS
{
  /* GRUB is picky about PT_LOAD layout/offsets. Keep boot-time sections in ONE segment. */
  boot  PT_LOAD FLAGS(0x7);  /* R W E */
  text  PT_LOAD FLAGS(0x5);  /* R E */
  rdata2 PT_LOAD FLAGS(0x4);
  dataw PT_LOAD FLAGS(0x6);  /* R W: .data + .bss in one phdr (GRUB/ELF) */
}

SECTIONS
{
  . = BOOT_LMA;
  .text.boot : {
    KEEP(*(.multiboot))
    *(.text.boot)
  } :boot
  . = ALIGN(0x1000);
  .rodata.boot : {
    *(.rodata.boot)
  } :boot
  . = ALIGN(0x1000);
  .bss.boot (NOLOAD) : {
    *(.bss.boot)
  } :boot
  boot_lma_end = .;

  /* High-half mapping in kernel.asm uses VA(base+X) -> PA(X) for first 1GiB.
     Therefore high-half section VAs must use X=LMA (not LMA-BOOT_LMA). */
  . = KERNEL_VMA_BASE + boot_lma_end;
  .text : AT(boot_lma_end) {
    *(.text)
    *(.text.*)
    /* clang -mcmodel=large */
    *(.ltext)
    *(.ltext.*)
  } :text
  __text_vend = .;
  . = ALIGN(0x10);
  .rodata : {
    *(.rodata)
    *(.rodata.*)
    *(.lrodata)
    *(.lrodata.*)
  } :rdata2
  . = ALIGN(0x10);
  .data : {
    *(.data)
    *(.data.*)
    *(.ldata)
    *(.ldata.*)
  } :dataw
  . = ALIGN(0x10);
  /* .lbss must live in the same phdr as .bss (one RW PT_LOAD, correct p_offset). */
  .bss (NOLOAD) : {
    *(.bss)
    *(.bss.*)
    *(.lbss)
    *(.lbss.*)
    *(COMMON)
  } :dataw

  . = ALIGN(0x1000);
  __kernel_end = .;
  __kernel_start = ADDR(.text);
  PROVIDE(_end = .);
  PROVIDE(_kernel_end = .);

  __kernel_lma_start = BOOT_LMA;
  __kernel_lma_end   = ALIGN(LOADADDR(.bss) + SIZEOF(.bss), 0x1000);
  __kernel_lma_size  = __kernel_lma_end - __kernel_lma_start;

  /* UEFI handoff contract (uefi_boot.h / bootx64.c). .text.boot must stay under
     one page so .bss.boot starts at 0x102000 and the handoff follows the 8-byte
     multiboot2_info_ptr. The MBI is no longer a fixed address inside the image:
     the loader AllocatePages() it and passes mbi_phys in the handoff.
     No ';' after ASSERT here: ld.lld rejects ASSERT(...); inside SECTIONS. */
  ASSERT(popcorn_uefi_handoff == 0x102008,
         "popcorn_uefi_handoff moved: must equal POPCORN_UEFI_HANDOFF_PHYS (0x102008)")
  ASSERT(uefi_start >= BOOT_LMA,
         "uefi_start below 0x100000: bootx64.c only accepts 0x100000 to 0x2FFFFF")
  ASSERT(uefi_start < 0x300000,
         "uefi_start at or above 0x300000: bootx64.c only accepts 0x100000 to 0x2FFFFF")
  ASSERT(__kernel_lma_size <= 0x10000000,
         "kernel image exceeds KERNEL_SPAN_MAX_BYTES (256 MiB) in bootx64.c")

  /DISCARD/ : {
    *(.eh_frame)
    *(.eh_frame_hdr)
  }
}
