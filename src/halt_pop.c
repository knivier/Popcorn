// src/halt_pop.c
#include "includes/console.h"
#include "includes/utils.h"

// Function to display the halt message with animation
void halt_pop_func(unsigned int start_pos) {
    (void)start_pos; // Unused parameter
    
    char* vidptr = (char*)0xb8000;
    const char* msg = "System Halted, Press Enter to Continue";

    // Clear the screen with green background
    for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
        vidptr[m] = ' ';
        vidptr[m + 1] = 0x20; // Green background with black text
    }

    unsigned int msg_len = 0;
    while (msg[msg_len] != '\0') msg_len++;
    
    unsigned int start_x = (80 - msg_len) / 2;
    unsigned int start_y = 12; // Middle of screen
    unsigned int pos = (start_y * 80 + start_x) * 2;
    
    for (unsigned int j = 0; j < msg_len; j++) {
        vidptr[pos + (j * 2)] = msg[j];
        vidptr[pos + (j * 2) + 1] = 0x2C; // Green background with red text
    }

    // Animation effect (simple blinking)
    for (int blink = 0; blink < 5; ++blink) {
        for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
            vidptr[m + 1] ^= 0x08; // Toggle blink bit
        }
        util_delay(500);
    }

    // Rainbow color animation
    for (int rainbow = 0; rainbow < 8; rainbow++) {
        for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
            vidptr[m + 1] = (rainbow << 4) | 0x0F;
        }
        util_delay(300);
    }

    // Fade to black
    for (int fade = 0x0F; fade >= 0; fade--) {
        for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
            vidptr[m + 1] = fade;
        }
        util_delay(100);
    }
}

// Define the PopModule structure for halt_pop
const struct {
    const char* name;
    const char* message;
    void (*pop_function)(unsigned int start_pos);
} halt_module = {
    .name = "halt",
    .message = "Displays a Green Screen of Death",
    .pop_function = halt_pop_func
};