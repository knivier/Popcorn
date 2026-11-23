/* CPU exception panic dump (COM1 + debugcon). Called from idt.asm stubs. */
#include "../includes/boot_fb.h"
#include <stddef.h>
#include <stdint.h>

/* Stack layout built by cpu_exception_common (idt.asm): saved GPRs (rax pushed first),
 * then the vector and error code pushed by the stub/CPU, then the CPU frame. */
typedef struct {
    uint64_t r15, r14, r13, r12, r11, r10, r9, r8;
    uint64_t rbp, rdi, rsi, rdx, rcx, rbx, rax;
    uint64_t vector;
    uint64_t error;
    uint64_t rip, cs, rflags, rsp, ss;
} exc_frame_t;

_Static_assert(offsetof(exc_frame_t, rax) == 112, "idt.asm PUSH_GPRS layout");
_Static_assert(offsetof(exc_frame_t, vector) == 120, "idt.asm exception vector slot");
_Static_assert(offsetof(exc_frame_t, rip) == 136, "idt.asm exception CPU frame");

/* default_cpu_exception (shared gate): vector and error-code presence unknown. */
#define EXC_VECTOR_UNKNOWN 0xFFFFu

void cpu_exception_panic(const exc_frame_t* f, uint64_t cr2);

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
        case 0:
            return "#DE";
        case 1:
            return "#DB";
        case 2:
            return "NMI";
        case 3:
            return "#BP";
        case 4:
            return "#OF";
        case 5:
            return "#BR";
        case 6:
            return "#UD";
        case 7:
            return "#NM";
        case 8:
            return "#DF";
        case 10:
            return "#TS";
        case 11:
            return "#NP";
        case 12:
            return "#SS";
        case 13:
            return "#GP";
        case 14:
            return "#PF";
        case 16:
            return "#MF";
        case 17:
            return "#AC";
        case 18:
            return "#MC";
        case 19:
            return "#XM";
        case 20:
            return "#VE";
        case 21:
            return "#CP";
        default:
            return "#EXC";
    }
}

static void exc_put_reg(const char* name, uint64_t v) {
    exc_puts(name);
    exc_put_hex64(v);
}

void cpu_exception_panic(const exc_frame_t* f, uint64_t cr2) {
    const uint64_t vector = f->vector;
    const uint64_t error = f->error;

    exc_puts("\r\n*** CPU EXCEPTION ");
    exc_puts(exc_name(vector));
    exc_puts(" ***\r\n");
    if (vector == EXC_VECTOR_UNKNOWN) {
        exc_puts("  (shared gate: vector unknown; frame may be shifted by an error code)\r\n");
    }
    exc_puts("  vector=");
    exc_put_hex64(vector);
    exc_puts("  error=");
    exc_put_hex64(error);
    exc_puts("\r\n  RIP=");
    exc_put_hex64(f->rip);
    exc_puts("  CS=");
    exc_put_hex64(f->cs);
    exc_puts("\r\n  RFLAGS=");
    exc_put_hex64(f->rflags);
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
    exc_put_reg("\r\n  RSP=", f->rsp);
    exc_put_reg("  SS=", f->ss);
    exc_put_reg("\r\n  RAX=", f->rax);
    exc_put_reg("  RBX=", f->rbx);
    exc_put_reg("  RCX=", f->rcx);
    exc_put_reg("\r\n  RDX=", f->rdx);
    exc_put_reg("  RSI=", f->rsi);
    exc_put_reg("  RDI=", f->rdi);
    exc_put_reg("\r\n  RBP=", f->rbp);
    exc_put_reg("  R8=", f->r8);
    exc_put_reg("  R9=", f->r9);
    exc_put_reg("\r\n  R10=", f->r10);
    exc_put_reg("  R11=", f->r11);
    exc_put_reg("  R12=", f->r12);
    exc_put_reg("\r\n  R13=", f->r13);
    exc_put_reg("  R14=", f->r14);
    exc_put_reg("  R15=", f->r15);
    exc_puts("\r\n*** halt ***\r\n");

    for (;;) {
        __asm__ volatile("cli; hlt");
    }
}
