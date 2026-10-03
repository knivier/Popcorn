// src/core/pop_module.c
#include "../includes/pop_module.h"
#include "../includes/console.h"

#define MAX_MODULES 10

static PopModule* modules[MAX_MODULES];
static int module_count = 0;

void register_pop_module(const PopModule* module) {
    if (module_count < MAX_MODULES) {
        modules[module_count++] = (PopModule*)module;
    }
}

void execute_all_pops(unsigned int start_pos) {
    unsigned int cols = console_cols();
    if (cols == 0) {
        cols = 80;
    }
    unsigned int current_pos = start_pos;
    unsigned int line_width = cols * 2;
    current_pos = (current_pos / line_width + 1) * line_width;

    for (int i = 0; i < module_count; i++) {
        const PopModule* module = modules[i];
        if (module && module->pop_function) {
            module->pop_function(current_pos);
            current_pos += line_width;
        }
    }
}
