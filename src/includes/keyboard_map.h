/* Scancode set 1 maps — http://www.osdever.net/bkerndev/Docs/keyboard.html */

#ifndef KEYBOARD_MAP_H
#define KEYBOARD_MAP_H

#define KBD_SCAN_LSHIFT   0x2A
#define KBD_SCAN_RSHIFT   0x36
#define KBD_SCAN_CAPS     0x3A

/* static const: safe to include from several translation units (kernel.c, dolphin_pop.c). */
static const unsigned char keyboard_map[128] =
{
    0,  27, '1', '2', '3', '4', '5', '6', '7', '8',
  '9', '0', '-', '=', '\b',
  '\t',
  'q', 'w', 'e', 'r',
  't', 'y', 'u', 'i', 'o', 'p', '[', ']', '\n',
    0, /* Ctrl */
  'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';',
 '\'', '`',   0, /* Left shift */
 '\\', 'z', 'x', 'c', 'v', 'b', 'n',
  'm', ',', '.', '/',   0, /* Right shift */
  '*',
    0, /* Alt */
  ' ',
    0, /* Caps lock */
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, /* F1-F10 */
    0, 0, 0, 0, 0, '-', 0, 0, 0, '+',
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
};

/* Shifted punctuation / digits; letters handled by case logic. */
static const unsigned char keyboard_map_shift[128] =
{
    0,  27, '!', '@', '#', '$', '%', '^', '&', '*',
  '(', ')', '_', '+', '\b',
  '\t',
  'Q', 'W', 'E', 'R',
  'T', 'Y', 'U', 'I', 'O', 'P', '{', '}', '\n',
    0,
  'A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L', ':',
 '\"', '~',   0,
 '|', 'Z', 'X', 'C', 'V', 'B', 'N',
  'M', '<', '>', '?',   0,
  '*',
    0,
  ' ',
    0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, '-', 0, 0, 0, '+',
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
};

/* Set-1 make code -> ASCII honouring Shift and Caps Lock (Caps XOR Shift for
 * letters, Shift only for everything else). 0 if the key has no character. */
static inline char kbd_scancode_to_char(unsigned char code, int shift, int caps) {
    if (code >= 128) {
        return 0;
    }
    char base = (char)keyboard_map[code];
    if (base >= 'a' && base <= 'z') {
        return ((caps != 0) != (shift != 0)) ? (char)(base - 'a' + 'A') : base;
    }
    return shift ? (char)keyboard_map_shift[code] : base;
}

/* Live modifier state, maintained by kmain's scancode loop (kernel.c). */
int kbd_shift_active(void);
int kbd_caps_active(void);

#endif
