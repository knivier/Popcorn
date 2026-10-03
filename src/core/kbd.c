#include "../includes/kbd.h"
#include "../includes/keyboard_queue.h"
#include "../includes/irq.h"
#include <stdint.h>
#include <stdbool.h>

#define KEYBOARD_DATA_PORT 0x60
#define KEYBOARD_STATUS_PORT 0x64

extern char read_port(unsigned short port);
extern void write_port(unsigned short port, unsigned char data);

#define KEY_QUEUE_CAP 256
static volatile unsigned char key_queue[KEY_QUEUE_CAP];
static volatile uint32_t key_queue_head;
static volatile uint32_t key_queue_tail;

static void key_queue_push(unsigned char scancode)
{
    uint32_t tail = key_queue_tail;
    uint32_t next = (tail + 1U) % KEY_QUEUE_CAP;
    uint32_t head = key_queue_head;
    if (next == head) {
        return; /* full: drop */
    }
    key_queue[tail] = scancode;
    key_queue_tail = next;
}

bool key_queue_pop(uint8_t* out)
{
    if (!out) {
        return false;
    }
    uint32_t head = key_queue_head;
    if (head == key_queue_tail) {
        return false;
    }
    *out = key_queue[head];
    key_queue_head = (head + 1U) % KEY_QUEUE_CAP;
    return true;
}

static void kbc_wait_input(void) {
    for (int i = 0; i < 100000; i++) {
        if ((read_port(KEYBOARD_STATUS_PORT) & 0x02) == 0) {
            return;
        }
    }
}

void kb_init(void)
{
    /* Enable PS/2 keyboard and drain stale bytes (ThinkPad/UEFI often skips IRQ1). */
    kbc_wait_input();
    write_port(KEYBOARD_STATUS_PORT, 0xAE);
    kbc_wait_input();
    while (read_port(KEYBOARD_STATUS_PORT) & 0x01) {
        (void)read_port(KEYBOARD_DATA_PORT);
    }
    irq_enable(1);
}

void keyboard_poll_ps2(void) {
    unsigned char status = read_port(KEYBOARD_STATUS_PORT);
    if (status & 0x01) {
        key_queue_push(read_port(KEYBOARD_DATA_PORT));
    }
}

void keyboard_handler_main(void) {
    unsigned char status = read_port(KEYBOARD_STATUS_PORT);
    if (status & 0x01) {
        unsigned char keycode = read_port(KEYBOARD_DATA_PORT);
        key_queue_push(keycode);
    }
    /* Master PIC: End of interrupt */
    write_port(0x20, 0x20);
}

