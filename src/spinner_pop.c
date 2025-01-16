// src/spinner_pop.c
#include "includes/pop_module.h"
#include "includes/spinner_pop.h"

#define SCREEN_WIDTH 80
#define SCREEN_HEIGHT 25

void delay(unsigned int milliseconds) {
    for (volatile unsigned int i = 0; i < milliseconds * 1000; i++);
}

void spinner_pop_func(unsigned int start_pos) {
    (void)start_pos;  // Avoid unused parameter warning
    char* vidptr = (char*)0xb8000;
    static const char spinner[] = {'|', '/', '-', '\\'};
    static const char* msg = "Running... ";
    static int spinner_state = 0;
    
    // Calculate the starting position for the bottom right corner
    unsigned int bottom_right_pos = (SCREEN_HEIGHT - 1) * SCREEN_WIDTH * 2 + (SCREEN_WIDTH - sizeof("Loading... ")) * 2;

    // Write "Loading..."
    for(int j = 0; msg[j] != '\0'; j++) {
        vidptr[bottom_right_pos + (j * 2)] = msg[j];
        vidptr[bottom_right_pos + (j * 2) + 1] = 0x0E;  // Yellow color
    }

    // Position for spinner (right after the message)
    unsigned int pos = bottom_right_pos + (sizeof("Loading... ") - 1) * 2;

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
