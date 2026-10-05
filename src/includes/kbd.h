#ifndef KBD_H
#define KBD_H

#include <stdint.h>
#include <stdbool.h>

/* Thin C surface — PS/2 I/O + queue are the Rust `kbd` drive (/dev/kbd). */
void kb_init(void);
void keyboard_poll_ps2(void);
void keyboard_handler_main(void);
bool kbd_read_scancode(uint8_t* out);
/* Compat: same as kbd_read_scancode (Dolphin). Prefer kbd_read_scancode. */
bool key_queue_pop(uint8_t* out);

#endif /* KBD_H */
