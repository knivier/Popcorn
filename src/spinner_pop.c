// src/spinner_pop.c
#include "includes/pop_module.h"
#include "includes/spinner_pop.h"
// Implement delay function directly
void delay(unsigned int milliseconds) {
    for (volatile unsigned int i = 0; i < milliseconds * 1000; i++);
}

void spinner_pop_func(unsigned int start_pos) {
    char* vidptr = (char*)0xb8000;
    static const char spinner[] = {'|', '/', '-', '\\'};
    static const char* msg = "Loading... ";
    unsigned int pos = start_pos;
    unsigned int j = 0;
    
    // Write "Loading..."
    while (msg[j] != '\0') {
        vidptr[pos] = msg[j];
        vidptr[pos + 1] = 0x0E;  // Yellow color
        j++;
        pos += 2;
    }
    
    // Animate spinner
    for (int i = 0; i < 20; i++) {
        vidptr[pos] = spinner[i % 4];
        vidptr[pos + 1] = 0x0E;
        delay(1);  // Slow it down to see animation
    }
}

const PopModule spinner_module = {
    .name = "spinner",
    .message = "Spinning loader animation",
    .pop_function = spinner_pop_func
};
