#include "../includes/utils.h"

// Common delay function used across modules
void util_delay(unsigned int milliseconds) {
    for (volatile unsigned int i = 0; i < milliseconds * 1000; i++);
}

