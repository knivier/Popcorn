/*
*  kernel.c
*/
#include "includes/pop_module.h"
extern const PopModule shimjapii_module;

void kmain(void)
{
    const char *str = "Popcorn v0.3 Popped!!!";
    char *vidptr = (char*)0xb8000;     //video mem begins here.
    unsigned int i = 0;
    unsigned int j = 0;

    /* this loops clears the screen
    * there are 25 lines each of 80 columns; each element takes 2 bytes */
    while(j < 80 * 25 * 2) {
        /* blank character */
        vidptr[j] = ' ';
        /* attribute-byte - light grey on black screen */
        vidptr[j+1] = 0x07;         
        j = j + 2;
    }

    j = 0;

    /* this loop writes the string to video memory */
    while(str[j] != '\0') {
        /* the character's ascii */
        vidptr[i] = str[j];
        /* attribute-byte: give character black bg and light grey fg */
        vidptr[i+1] = 0x07;
        ++j;
        i = i + 2;
    }

    // Register and execute the shimjapii module
    register_pop_module(&shimjapii_module);
    execute_all_pops(i);  // Pass the current position after printing the first message

    return;
}