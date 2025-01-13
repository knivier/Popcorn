// kernel.c
#include "shimjapii.h"  // Include the header for shimjapii.c

void delay(int count) {
    while (count--) {
        for (int i = 0; i < 1000000; i++) {
            // Do nothing, just waste some time
        }
    }
}

void kmain(void) {
    const char *str = "[✓] Popcorn v0.1-unstable Booted, awaiting pops";
    char *vidptr = (char*)0xb8000;  // Video memory
    unsigned int i = 0, j = 0;

    // Clear the screen
    while (j < 80 * 25 * 2) {
        vidptr[j] = ' ';
        vidptr[j + 1] = 0x07;  // Light grey on black background
        j = j + 2;
    }

    // Write "Popcorn v0.1-unstable Booted, awaiting pops" to video memory
    j = 0;
    while (str[j] != '\0') {
        vidptr[i] = str[j];
        vidptr[i + 1] = 0x07;  // Light grey on black background
        ++j;
        i = i + 2;
    }

    // Wait for one second
    delay(1);

    // Call the function from shimjapii.c to print the message
    shimjapii_popped(i);  // Pass the current position so shimjapii_popped() can continue from there
}
