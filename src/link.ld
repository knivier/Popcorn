/*
*  link.ld - 64-bit version
*/
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(start)
SECTIONS
{
   . = 0x100000;
   
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
}

PHDRS
{
   code PT_LOAD FLAGS(5); /* Read + Execute */
   data PT_LOAD FLAGS(6); /* Read + Write */
}