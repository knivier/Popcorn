#ifndef SPINNER_POP_H
#define SPINNER_POP_H

#include "pop_module.h"

// Declare the delay function needed by spinner_pop.c
void delay(unsigned int count);

// Declare the spinner function
void spinner_pop_func(unsigned int start_pos);

// Declare the module structure as external
extern const PopModule spinner_module;

#endif /* SPINNER_POP_H */