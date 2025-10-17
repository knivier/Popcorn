// src/spinner_pop.c
#include "includes/pop_module.h"
#include "includes/spinner_pop.h"
#include "includes/console.h"

#define SCREEN_WIDTH 80
#define SCREEN_HEIGHT 25

void delay(unsigned int milliseconds) {
    for (volatile unsigned int i = 0; i < milliseconds * 1000; i++);
}

// Access console state to save/restore cursor
extern ConsoleState console_state;

void spinner_pop_func(unsigned int start_pos) {
    (void)start_pos;  // Avoid unused parameter warning
    static const char spinner[] = {'|', '/', '-', '\\'};
    static const char* msg = "Running... ";
    static int spinner_state = 0;

    // Save current console state
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;

    // Draw near the top (row 3) so it remains static below header
    unsigned int y = 3; // row index after header (which uses rows 0-2)
    unsigned int msg_len = (unsigned int)sizeof("Running... ") - 1;
    unsigned int msg_x = SCREEN_WIDTH - msg_len - 2; // leave space for spinner and a space

    // Write the label
    console_set_cursor(msg_x, y);
    console_print_color(msg, CONSOLE_WARNING_COLOR);

    // Write the spinner glyph right after the message
    console_set_cursor(msg_x + (int)msg_len + 1, y);
    char ch[2];
    ch[0] = spinner[spinner_state];
    ch[1] = '\0';
    console_print_color(ch, CONSOLE_WARNING_COLOR);

    // Restore previous cursor/color so normal typing is unaffected
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);

    // Update spinner state for next call
    spinner_state = (spinner_state + 1) % 4;

    // Very small delay to avoid interfering with typing
    delay(10);
}

const PopModule spinner_module = {
    .name = "spinner",
    .message = "Spinning loader animation",
    .pop_function = spinner_pop_func
};
