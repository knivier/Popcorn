// src/spinner_pop.c
#include "includes/pop_module.h"
#include "includes/spinner_pop.h"

void delay(unsigned int milliseconds) {
    for (volatile unsigned int i = 0; i < milliseconds * 1000; i++);
}

void spinner_pop_func(unsigned int start_pos) {
    char* vidptr = (char*)0xb8000;
    static const char spinner[] = {'|', '/', '-', '\\'};
    static const char* msg = "Loading... ";
    static int spinner_state = 0;
    
    // Write "Loading..."
    for(int j = 0; msg[j] != '\0'; j++) {
        vidptr[start_pos + (j * 2)] = msg[j];
        vidptr[start_pos + (j * 2) + 1] = 0x0E;  // Yellow color
    }

    // Position for spinner (right after the message)
    unsigned int pos = start_pos + (sizeof("Loading... ") - 1) * 2;

    // Update spinner
    vidptr[pos] = spinner[spinner_state];
    vidptr[pos + 1] = 0x0E;
    
    // Update spinner state for next call
    spinner_state = (spinner_state + 1) % 4;

    // Add small delay
    delay(1000);
}

const PopModule spinner_module = {
    .name = "spinner",
    .message = "Spinning loader animation",
    .pop_function = spinner_pop_func
};
