#include "../includes/pop_module.h"
#include "../includes/console.h"
#include "../includes/timer.h"
#include "../includes/utils.h"

// External timer functions
extern uint64_t timer_get_ticks(void);
extern uint64_t timer_get_uptime_ms(void);

// Access console state to save/restore cursor
extern ConsoleState console_state;

void uptime_pop_func(unsigned int start_pos) {
    (void)start_pos;
    
    char buffer[64];
    char tmp[10];
    int i = 0;

    // Construct the string "Ticks: "
    const char* msg = "Ticks: ";
    while (msg[i] != '\0') {
        buffer[i] = msg[i];
        ++i;
    }

    // Get tick count from timer system
    uint64_t tick_count = timer_get_ticks();
    
    // Convert tick_counter to string and append
    int_to_str((int)tick_count, tmp);
    int j = 0;
    while (tmp[j] != '\0') {
        buffer[i++] = tmp[j++];
    }
    buffer[i] = '\0';

    // Save and restore console state so cursor doesn't move
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;

    console_set_cursor(1, 0);
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
    return (unsigned int)timer_get_ticks();
}
