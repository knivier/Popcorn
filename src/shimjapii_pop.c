// src/shimjapii_pop.c
#include "includes/pop_module.h"
#include "includes/console.h"

extern ConsoleState console_state;

void shimjapii_pop_func(unsigned int start_pos) {
    (void)start_pos; 
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;
    
    const char* msg = "Shimjapii popped!!!!";
    unsigned int msg_len = 0;
    while (msg[msg_len] != '\0') msg_len++;
    
    console_set_cursor(80 - msg_len - 1, 23);
    console_print_color(msg, CONSOLE_SUCCESS_COLOR);
    
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);
}

const PopModule shimjapii_module = {
    .name = "shimjapii",
    .message = "Shimjapii popped!!!",
    .pop_function = shimjapii_pop_func
};