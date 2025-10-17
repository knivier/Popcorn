#include "includes/pop_module.h"
#include "includes/console.h"

// Global tick counter
static unsigned long long tick_counter = 0;

void int_to_str(int value, char* buffer) {
    char temp[10];
    int i = 0, j = 0;

    if (value == 0) {
        buffer[i++] = '0';
    } else {
        while (value > 0) {
            temp[j++] = (value % 10) + '0';
            value /= 10;
        }
        while (j > 0) {
            buffer[i++] = temp[--j];
        }
    }
    buffer[i] = '\0';
}

// Access console state to save/restore cursor
extern ConsoleState console_state;

void uptime_pop_func(unsigned int start_pos) {
    (void)start_pos;
    // Increment the tick counter
    tick_counter++;

    char buffer[64];
    char tmp[10];
    int i = 0;

    // Construct the string "Ticks: "
    const char* msg = "Ticks: ";
    while (msg[i] != '\0') {
        buffer[i] = msg[i];
        ++i;
    }

    // Convert tick_counter to string and append
    int_to_str(tick_counter, tmp);
    int j = 0;
    while (tmp[j] != '\0') {
        buffer[i++] = tmp[j++];
    }
    buffer[i] = '\0';

    // Save and restore console state so cursor doesn't move
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;

    // Place at top-left after header (row 3, col 1)
    console_set_cursor(1, 3);
    console_print_color(buffer, CONSOLE_INFO_COLOR);

    // Restore state
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);
}

const PopModule uptime_module = {
    .name = "uptime",
    .message = "Displays the tick counter",
    .pop_function = uptime_pop_func
};

unsigned int get_tick_count(void) {
    return tick_counter;
}