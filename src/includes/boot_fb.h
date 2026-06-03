#ifndef BOOT_FB_H
#define BOOT_FB_H

#include <stdint.h>

/* Popcorn console / GOP backdrop (BGR 0xRRGGBB). */
#define POPCORN_FB_BG_RGB 0x001018u

/* Mark identity-map 2 MiB pages covering [phys, phys+len) as uncached (UC). */
void boot_identity_uncache_range(uint64_t phys, uint64_t len);

/* Solid fill using BGRx or RGBx layout (byte0..2 = B,G,R for typical UEFI GOP). */
void boot_fb_solid_fill(volatile uint8_t* base, uint32_t pitch, uint32_t width,
                        uint32_t height, uint8_t bytespp, uint32_t rgb);

void boot_serial_putc(char c);
void boot_diag_caps_led_on(void);

#endif
