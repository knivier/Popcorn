#include "../includes/kbd.h"
#include "../includes/keyboard_queue.h"
#include "../includes/keyboard_map.h"
#include "../includes/irq.h"
#include "../includes/driver_abi.h"
#include "../includes/device.h"
#include <stdint.h>
#include <stdbool.h>

static const unsigned char keyboard_map[128] = {
    0,  27, '1', '2', '3', '4', '5', '6', '7', '8',
  '9', '0', '-', '=', '\b',
  '\t',
  'q', 'w', 'e', 'r',
  't', 'y', 'u', 'i', 'o', 'p', '[', ']', '\n',
    0,
  'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';',
 '\'', '`',   0,
 '\\', 'z', 'x', 'c', 'v', 'b', 'n',
  'm', ',', '.', '/',   0,
  '*',
    0,
  ' ',
    0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, '-', 0, 0, 0, '+',
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
};

static const unsigned char keyboard_map_shift[128] = {
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

char kbd_scancode_to_char(unsigned char code, int shift, int caps) {
    char base;
    if (code >= 128) {
        return 0;
    }
    base = (char)keyboard_map[code];
    if (base >= 'a' && base <= 'z') {
        return ((caps != 0) != (shift != 0)) ? (char)(base - 'a' + 'A') : base;
    }
    return shift ? (char)keyboard_map_shift[code] : base;
}

/* PIC EOI only — PS/2 ports + queue live in the Rust kbd drive. */

static Device* kbd_dev(void) {
    return device_find("kbd");
}

/* Read one scancode through the /dev/kbd device ops (not a private queue API). */
static bool kbd_dev_read(uint8_t* out) {
    Device* dev;
    if (!out) {
        return false;
    }
    rust_kbd_poll();
    dev = kbd_dev();
    if (!dev || !dev->ops || !dev->ops->read) {
        return rust_kbd_pop(out) != 0;
    }
    return dev->ops->read(dev, out, 1) == 1;
}

void kb_init(void)
{
    (void)init_drive("kbd");
    irq_enable(1);
}

void keyboard_poll_ps2(void) {
    rust_kbd_poll();
}

void keyboard_handler_main(void) {
    rust_kbd_irq();
    pic_send_eoi(1);
}

bool key_queue_pop(uint8_t* out)
{
    /* Compat name used by Dolphin; path is /dev/kbd. */
    return kbd_dev_read(out);
}

bool kbd_read_scancode(uint8_t* out)
{
    return kbd_dev_read(out);
}
