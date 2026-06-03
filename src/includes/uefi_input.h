#ifndef UEFI_INPUT_H
#define UEFI_INPUT_H

#include <stdbool.h>
#include <stdint.h>
#include <stdint.h>

/* Poll firmware SimpleTextIn (USB keyboard on ThinkPad). Returns true if a key was read. */
bool uefi_input_poll(uint8_t* key_out, bool* is_scancode);

bool uefi_input_available(void);

uint64_t uefi_input_poll_attempts(void);

#endif
