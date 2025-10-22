// src/pops/halt_pop.c
#include "../includes/console.h"
#include "../includes/utils.h"

// Access VGA memory for special effects
#define VGA_MEMORY ((char*)0xb8000)

// Function to display the halt message with animation
void halt_pop_func(unsigned int start_pos) {
    (void)start_pos; // Unused parameter
    
    const char* msg = "System Halted, Press Enter to Continue";
    
    // Save console state
    extern ConsoleState console_state;
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;

    // Clear the screen with green background using console system
    console_clear();
    
    // Fill screen with green background
    char* vidptr = VGA_MEMORY;
    for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
        vidptr[m] = ' ';
        vidptr[m + 1] = 0x20; // Green background with black text
    }

    // Display centered message using console system
    unsigned int msg_len = 0;
    while (msg[msg_len] != '\0') msg_len++;
    
    unsigned int start_x = (80 - msg_len) / 2;
    unsigned int start_y = 12; // Middle of screen
    
    console_set_cursor(start_x, start_y);
    console_print_color(msg, 0x2C); // Green background with red text

    // Animation effect
    for (int blink = 0; blink < 5; ++blink) {
        for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
            vidptr[m + 1] ^= 0x08; // Toggle blink bit
        }
        util_delay(500);
    }

    // Rainbow color animation
    for (int rainbow = 0; rainbow < 8; rainbow++) {
        for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
            vidptr[m + 1] = (unsigned char)((rainbow << 4) | 0x0F);
        }
        util_delay(300);
    }

    // Fade to black
    for (int fade = 0x0F; fade >= 0; fade--) {
        for (unsigned int m = 0; m < 80 * 25 * 2; m += 2) {
            vidptr[m + 1] = (unsigned char)fade;
        }
        util_delay(100);
    }
    
    // Restore console state
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);
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
