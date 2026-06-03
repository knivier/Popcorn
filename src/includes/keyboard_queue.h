#ifndef KEYBOARD_QUEUE_H
#define KEYBOARD_QUEUE_H

#include <stdbool.h>
#include <stdint.h>

/* IRQ handler pushes scancodes; consumers (kmain, halt, Dolphin) pop. */
bool key_queue_pop(uint8_t* out);

#endif
