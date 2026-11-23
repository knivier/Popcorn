/* CPU exceptions (vectors 0x00-0x1F), reached through isr_dispatch with a PopFrame.
 * #DB and #BP are reported and resumed; every other exception in Ring 0 is fatal:
 * a full dump on COM1 + debugcon, then halt. */
#include "../includes/boot_fb.h"
#include "../includes/exception.h"
#include "../includes/scheduler.h"
#include <stddef.h>
#include <stdint.h>

static uint64_t g_breakpoints;
static uint64_t g_debug_traps;

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

static void exc_put_dec(uint64_t v) {
    char buf[24];
    int n = 0;
    do {
        buf[n++] = (char)('0' + (v % 10u));
        v /= 10u;
    } while (v && n < (int)sizeof buf);
    while (n > 0) {
        exc_putc(buf[--n]);
    }
}

const char* exception_name(uint64_t vector) {
    switch (vector) {
        case 0: return "#DE";
        case 1: return "#DB";
        case 2: return "NMI";
        case 3: return "#BP";
        case 4: return "#OF";
        case 5: return "#BR";
        case 6: return "#UD";
        case 7: return "#NM";
        case 8: return "#DF";
        case 10: return "#TS";
        case 11: return "#NP";
        case 12: return "#SS";
        case 13: return "#GP";
        case 14: return "#PF";
        case 16: return "#MF";
        case 17: return "#AC";
        case 18: return "#MC";
        case 19: return "#XM";
        case 20: return "#VE";
        case 21: return "#CP";
        default: return "#EXC";
    }
}

static void exc_put_reg(const char* name, uint64_t v) {
    exc_puts(name);
    exc_put_hex64(v);
}

/* Only dereference stack words that sit in mapped kernel memory: the 64 GiB
 * identity map or the high half. A wild RSP must not fault inside the dump. */
static int exc_stack_readable(uint64_t rsp) {
    if (rsp & 7u) {
        return 0;
    }
    if (rsp >= 0x1000u && rsp + 64u < (64ULL << 30)) {
        return 1;
    }
    return rsp >= 0xFFFF800000000000ULL && rsp + 64u > rsp;
}

static void __attribute__((noreturn)) exception_fatal(const PopFrame* f) {
    const uint64_t vector = f->vector;
    const uint64_t error = f->error;
    uint64_t cr2, cr3;
    __asm__ volatile("mov %%cr2, %0" : "=r"(cr2));
    __asm__ volatile("mov %%cr3, %0" : "=r"(cr3));

    exc_puts("\r\n*** CPU EXCEPTION ");
    exc_puts(exception_name(vector));
    exc_puts(" ***\r\n");
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
    exc_put_reg("\r\n  CR3=", cr3);
    {
        TaskStruct* t = scheduler_get_current_task();
        exc_puts("  task=");
        if (t) {
            exc_puts("pid ");
            exc_put_dec(t->pid);
        } else {
            exc_puts("none");
        }
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
    if (exc_stack_readable(f->rsp)) {
        const uint64_t* sp = (const uint64_t*)(uintptr_t)f->rsp;
        exc_puts("\r\n  stack:");
        for (int i = 0; i < 8; i++) {
            exc_puts(i == 4 ? "\r\n        " : " ");
            exc_put_hex64(sp[i]);
        }
    }
    exc_puts("\r\n*** halt ***\r\n");

    for (;;) {
        __asm__ volatile("cli; hlt");
    }
}

PopFrame* exception_handle(PopFrame* f) {
    switch (f->vector) {
        case 1: /* #DB: trap after the instruction; nothing to fix up. */
            g_debug_traps++;
            exc_puts("\r\n[exception] #DB RIP=");
            exc_put_hex64(f->rip);
            exc_puts(" (resumed)\r\n");
            return f;
        case 3: /* #BP: int3 is a trap; RIP already points past it. */
            g_breakpoints++;
            exc_puts("\r\n[exception] #BP RIP=");
            exc_put_hex64(f->rip);
            exc_puts(" (resumed)\r\n");
            return f;
        default:
            /* Ring 0 has no one to blame but itself. (Ring-3 faults will kill the
             * task instead, once user tasks exist.) */
            exception_fatal(f);
    }
}

uint64_t exception_breakpoint_count(void) {
    return g_breakpoints;
}

uint64_t exception_debug_trap_count(void) {
    return g_debug_traps;
}
