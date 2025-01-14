// src/kernel.c
#include "includes/pop_module.h"

void delay(int count) {
    while (count--) {
        for (int i = 0; i < 1000000; i++) {
            // Do nothing, just waste some time
        }
    }
}

void kmain(void) {
    const char* str = "[✓] Popcorn v0.2-unstable Booted, awaiting pops";
    char* vidptr = (char*)0xb8000;
    unsigned int i = 0, j = 0;

    // Clear screen
    while (j < 80 * 25 * 2) {
        vidptr[j] = ' ';
        vidptr[j + 1] = 0x07;
        j = j + 2;
    }

    // Write boot message
    j = 0;
    while (str[j] != '\0') {
        vidptr[i] = str[j];
        vidptr[i + 1] = 0x07;
        ++j;
        i = i + 2;
    }

    // Register the Shimjapii module
    extern const PopModule shimjapii_module;
    register_pop_module(&shimjapii_module);

    // Wait a second
    delay(1);

    // Execute all registered pop modules
    execute_all_pops(i);
}
