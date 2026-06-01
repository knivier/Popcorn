/*
 * 64-bit kernel: physical load 1 MiB, virtual base 0xFFFF800000000000 (high half).
 * Boot code + early data live at 0x100000 LMA; C and most asm at VA = (high base + (LMA - 1M)).
 * GRUB: ELF entry = start, still at LMA 0x100000.
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
  } :text
  __text_vend = .;
  . = ALIGN(0x10);
  .rodata : {
    *(.rodata)
    *(.rodata.*)
  } :rdata2
  . = ALIGN(0x10);
  .data : {
    *(.data)
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

  /DISCARD/ : {
    *(.eh_frame)
    *(.eh_frame_hdr)
  }
}
