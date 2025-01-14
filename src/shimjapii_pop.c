// src/shimjapii_pop.c
#include "includes/pop_module.h"

void shimjapii_pop_func(unsigned int start_pos) {
    char* vidptr = (char*)0xb8000;
    const char* msg = "Shimjapii popped!";
    unsigned int i = start_pos;
    unsigned int j = 0;
    
    while (msg[j] != '\0') {
        vidptr[i] = msg[j];
        vidptr[i + 1] = 0x07;
        ++j;
        i = i + 2;
    }
}

const PopModule shimjapii_module = {
    .name = "shimjapii",
    .message = "Shimjapii popped!",
    .pop_function = shimjapii_pop_func
};