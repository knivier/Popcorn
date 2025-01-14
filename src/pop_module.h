// src/pop_module.h
#ifndef POP_MODULE_H
#define POP_MODULE_H

// Structure to define a pop module
typedef struct {
    const char* name;
    const char* message;
    void (*pop_function)(unsigned int start_pos);
} PopModule;

// Function to register a pop module
void register_pop_module(const PopModule* module);

// Function to execute all registered pops
void execute_all_pops(unsigned int start_pos);

#endif
