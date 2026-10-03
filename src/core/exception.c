/* CPU exception panic dump (COM1 + debugcon). Called from kernel.asm stubs. */
#include "../includes/boot_fb.h"
#include <stdint.h>

static void exc_putc(char c) {
    boot_serial_putc(c);
}

static void exc_puts(const char* s) {
    while (s && *s) {
        exc_putc(*s++);
    }
}

static void exc_put_hex64(uint64_t v) {
    static const char hex[] = "0123456789abcdef";
    exc_puts("0x");
    for (int i = 60; i >= 0; i -= 4) {
        exc_putc(hex[(v >> (unsigned)i) & 0xFu]);
    }
}

static const char* exc_name(uint64_t vector) {
    switch (vector) {
        case 8:
            return "#DF";
        case 13:
            return "#GP";
        case 14:
            return "#PF";
        default:
            return "#EXC";
    }
}

void cpu_exception_panic(uint64_t vector, uint64_t error, uint64_t rip, uint64_t cs,
                         uint64_t rflags, uint64_t cr2) {
    exc_puts("\r\n*** CPU EXCEPTION ");
    exc_puts(exc_name(vector));
    exc_puts(" ***\r\n");
    exc_puts("  vector=");
    exc_put_hex64(vector);
    exc_puts("  error=");
    exc_put_hex64(error);
    exc_puts("\r\n  RIP=");
    exc_put_hex64(rip);
    exc_puts("  CS=");
    exc_put_hex64(cs);
    exc_puts("\r\n  RFLAGS=");
    exc_put_hex64(rflags);
    if (vector == 14u) {
        exc_puts("\r\n  CR2=");
        exc_put_hex64(cr2);
        exc_puts("  (P=");
        exc_putc((error & 1u) ? '1' : '0');
        exc_puts(" W=");
        exc_putc((error & 2u) ? '1' : '0');
        exc_puts(" U=");
        exc_putc((error & 4u) ? '1' : '0');
        exc_puts(" I/D=");
        exc_putc((error & 16u) ? '1' : '0');
        exc_puts(")");
    }
    exc_puts("\r\n*** halt ***\r\n");

    for (;;) {
        __asm__ volatile("cli; hlt");
    }
}
