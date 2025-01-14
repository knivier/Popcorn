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

    while(j < 80 * 25 * 2) {
        vidptr[j] = ' ';
        vidptr[j+1] = 0x00;         
        j = j + 2;
    }

    j = 0;

    while(str[j] != '\0') {
        vidptr[i] = str[j];
        vidptr[i+1] = 0x02;  // Changed to 0x02 for green color
        ++j;
        i = i + 2;
    }

    // Register and execute the shimjapii module
    register_pop_module(&shimjapii_module);
    execute_all_pops(i);  
    return;
}