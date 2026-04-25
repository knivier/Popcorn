/*
*  link.ld - 64-bit version
*/
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(start)
SECTIONS
{
   . = 0x100000;
   __kernel_start = .;
   
   .multiboot : {
      *(.multiboot)
   } :code
   
   .text : {
      *(.text)
   } :code
   
   .rodata : {
      *(.rodata*)
   } :data
   
   .data : {
      *(.data)
   } :data
   
   .bss : {
      *(.bss)
      *(COMMON)
   } :data

   /* So kernel code can reserve the loaded image, BSS, and bitmap. */
   . = ALIGN(0x1000);
   __kernel_end = .;
   PROVIDE(_end = .);
   PROVIDE(_kernel_end = .);
}

PHDRS
{
   code PT_LOAD FLAGS(5); /* Read + Execute */
   data PT_LOAD FLAGS(6); /* Read + Write */
}