#include "../includes/kbd.h"
#include "../includes/keyboard_queue.h"
#include "../includes/irq.h"
#include "../includes/driver_abi.h"
#include "../includes/device.h"
#include <stdint.h>
#include <stdbool.h>

/* PIC EOI only — PS/2 ports + queue live in the Rust kbd drive. */
extern void write_port(unsigned short port, unsigned char data);

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
    write_port(0x20, 0x20); /* master PIC EOI */
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
