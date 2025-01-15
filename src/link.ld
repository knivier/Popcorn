/*
*  link.ld
*/
OUTPUT_FORMAT(elf32-i386)
ENTRY(start)
SECTIONS
{
   . = 0x100000;
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
   } :data
}

PHDRS
{
   code PT_LOAD FLAGS(5); /* Read + Execute */
   data PT_LOAD FLAGS(6); /* Read + Write */
}