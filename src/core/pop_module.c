// src/core/pop_module.c
#include "../includes/pop_module.h"

#define MAX_MODULES 10

/*
 * Registration catalog only. Rust pops/registry.rs registers each pop here
 * (via rust_pops_register from init_show_modules); nothing iterates the table
 * to run pop_function. Pops are driven from their own call sites (shell
 * commands, drives, the Dolphin editor), not from this registry.
 */
static PopModule* modules[MAX_MODULES];
static int module_count = 0;

void register_pop_module(const PopModule* module) {
    if (module_count < MAX_MODULES) {
        modules[module_count++] = (PopModule*)module;
    }
}
