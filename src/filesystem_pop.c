#include "includes/pop_module.h"

// Function to initialize the file system
void filesystem_pop_func(unsigned int start_pos) {
    char* vidptr = (char*)0xb8000;
    const char* msg = "Filesystem Initialized";
    unsigned int i = start_pos;
    unsigned int j = 0;

    // Display the initialization message on the screen
    while (msg[j] != '\0') {
        vidptr[i] = msg[j];
        vidptr[i + 1] = 0x02;  // Green color
        ++j;
        i = i + 2;
    }

    // Here you can add additional file system initialization code
}

const PopModule filesystem_module = {
    .name = "filesystem",
    .message = "Filesystem Initialized",
    .pop_function = filesystem_pop_func
};