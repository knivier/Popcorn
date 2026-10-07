/* Scancode set 1 maps — http://www.osdever.net/bkerndev/Docs/keyboard.html */

#ifndef KEYBOARD_MAP_H
#define KEYBOARD_MAP_H

#define KBD_SCAN_LSHIFT   0x2A
#define KBD_SCAN_RSHIFT   0x36
#define KBD_SCAN_CAPS     0x3A

/* Set-1 make code -> ASCII honouring Shift and Caps Lock (Caps XOR Shift for
 * letters, Shift only for everything else). 0 if the key has no character.
 * Implemented in kbd.c so Rust (and C) can call it via the kernel ABI. */
char kbd_scancode_to_char(unsigned char code, int shift, int caps);

/* Live modifier state, maintained by kmain's scancode loop (kernel.c). */
int kbd_shift_active(void);
int kbd_caps_active(void);

#endif
