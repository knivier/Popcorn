#ifndef BOOT_FB_H
#define BOOT_FB_H

#include <stdint.h>

/* Popcorn console / GOP backdrop (BGR 0xRRGGBB). */
#define POPCORN_FB_BG_RGB 0x001018u

/* Mark identity-map 2 MiB pages covering [phys, phys+len) as uncached (UC). */
void boot_identity_uncache_range(uint64_t phys, uint64_t len);

void boot_serial_putc(char c);
void boot_diag_caps_led_on(void);

#endif
