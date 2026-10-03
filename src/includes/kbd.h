#ifndef KBD_H
#define KBD_H

#include <stdint.h>
#include <stdbool.h>

void kb_init(void);
void keyboard_poll_ps2(void);
void keyboard_handler_main(void);
bool key_queue_pop(uint8_t* out);

#endif /* KBD_H */
