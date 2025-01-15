#include "includes/pop_module.h"

// Global tick counter
static unsigned int tick_counter = 0;

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

void uptime_pop_func(unsigned int start_pos) {
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

    char* vidptr = (char*)0xb8000;
    j = 0;

    while (buffer[j] != '\0') {
        vidptr[start_pos] = buffer[j];
        vidptr[start_pos + 1] = 0x07;  // White color
        ++j;
        start_pos += 2;
    }
}

const PopModule uptime_module = {
    .name = "uptime",
    .message = "Displays the tick counter",
    .pop_function = uptime_pop_func
};