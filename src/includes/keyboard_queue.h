#ifndef KEYBOARD_QUEUE_H
#define KEYBOARD_QUEUE_H

#include <stdbool.h>
#include <stdint.h>

/* Scancodes from Rust /dev/kbd; consumers (kmain, Dolphin) pop. */
bool key_queue_pop(uint8_t* out);

#endif
