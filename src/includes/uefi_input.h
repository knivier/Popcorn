#ifndef UEFI_INPUT_H
#define UEFI_INPUT_H

#include <stdbool.h>
#include <stdint.h>

/* Poll firmware SimpleTextIn (USB keyboard on ThinkPad). Returns true if a key was read.
 * Only live while Boot Services are up; the current loader always exits them, so
 * uefi_input_available() returns false on every boot today. */
bool uefi_input_poll(uint8_t* key_out, bool* is_scancode);

bool uefi_input_available(void);

uint64_t uefi_input_poll_attempts(void);

#endif
