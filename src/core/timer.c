// src/core/timer.c
#include "../includes/timer.h"
#include "../includes/console.h"
#include "../includes/irq.h"
#include "../includes/cpu_pop.h"
#include <stddef.h>

// External port I/O functions
extern void write_port(unsigned short port, unsigned char data);
extern char read_port(unsigned short port);

// Global timer state
TimerState global_timer = {0};
static uint16_t pit_divisor = 11932;
static bool pit_poll_mode = false;
static uint64_t timer_tsc0; /* TSC at timer_init, for timer_catch_up */

static uint64_t timer_rdtsc(void) {
    uint32_t lo, hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((uint64_t)hi << 32) | lo;
}

/*
 * Boot runs for a while (USB enumeration, POPCORN.LOG, driver probes) between
 * timer_init and the first IRQ0 with interrupts off, so those periods never
 * became ticks and uptime ran ~2 s slow on the T14s. When ticking starts, set
 * the count to the time that really passed, measured with the TSC. Only ever
 * moves the clock forward; skipped if the TSC frequency is unknown.
 */
static void timer_catch_up(void) {
    const CPUFrequency* f = cpu_get_frequency();
    uint64_t hz = f ? f->tsc_hz : 0;
    if (hz == 0 || timer_tsc0 == 0 || global_timer.frequency == 0) {
        return;
    }
    uint64_t d = timer_rdtsc() - timer_tsc0;
    uint64_t due = (d / hz) * global_timer.frequency +
                   ((d % hz) * global_timer.frequency) / hz;
    if (due > global_timer.ticks) {
        global_timer.ticks = due;
    }
}

// Initialize PIT (Programmable Interval Timer)
void timer_init(uint32_t frequency_hz) {
    if (frequency_hz == 0) {
        frequency_hz = TIMER_FREQUENCY;
    }
    // Calculate divisor for desired frequency (16-bit reload; 0 would mean 65536)
    uint32_t divisor = PIT_FREQUENCY / frequency_hz;
    if (divisor < 2) {
        divisor = 2;
    } else if (divisor > 0xFFFF) {
        divisor = 0xFFFF;
    }
    pit_divisor = (uint16_t)divisor;

    /*
     * Channel 0, mode 2 (rate generator), binary. IRQ0 still fires once per period,
     * and the counter counts down by 1 per input clock and reloads once per period,
     * so timer_poll() can turn counter deltas into whole periods (mode 3 counts by 2
     * and reloads twice per period).
     */
    write_port(0x43, 0x34);
    write_port(0x40, pit_divisor & 0xFF);        // Low byte
    write_port(0x40, (pit_divisor >> 8) & 0xFF); // High byte
    
    // Initialize timer state
    global_timer.ticks = 0;
    timer_tsc0 = timer_rdtsc();
    global_timer.frequency = frequency_hz;
    global_timer.is_active = false;
    global_timer.tick_handler = NULL;
    
    console_println_color("Timer initialized", CONSOLE_SUCCESS_COLOR);
}

// Timer interrupt handler (IRQ0, via isr_dispatch -> irq_dispatch)
void timer_interrupt_handler(void) {
    global_timer.ticks++;
    global_timer.irq_ticks++;

    /*
     * EOI first. scheduler_tick only requests a switch; it happens when isr_dispatch
     * returns another task's frame. IRQ gates keep IF=0 until iretq, so the early EOI
     * cannot nest IRQ0 on this stack.
     */
    pic_send_eoi(0);

    /* During USB BOT / xHCI: EOI only — never run the scheduler mid-transfer. */
    if (!irq_quiet_active() && global_timer.tick_handler) {
        global_timer.tick_handler();
    }
}

// Enable timer interrupts
void timer_enable(void) {
    timer_catch_up();
    irq_enable(0);
    global_timer.is_active = true;
    pit_poll_mode = false;
}

/* Clear pending PIC IRQs before sti (UEFI/QEMU may leave IRR latched). */
void pic_acknowledge_pending(void) {
    for (int i = 0; i < 4; i++) {
        write_port(0x20, 0x20);
        write_port(0xA0, 0x20);
    }
    write_port(0x20, 0x0B);
    (void)read_port(0x20);
    write_port(0xA0, 0x0B);
    (void)read_port(0xA0);
}

// Disable timer interrupts
void timer_disable(void) {
    irq_disable(0);
    global_timer.is_active = false;
    pit_poll_mode = false;
}

void timer_enable_poll(void) {
    timer_disable();
    timer_catch_up();
    pit_poll_mode = true;
    global_timer.is_active = true;
}

static uint16_t pit_latch_count0(void) {
    write_port(0x43, 0x00);
    uint8_t lo = (uint8_t)read_port(0x40);
    uint8_t hi = (uint8_t)read_port(0x40);
    return (uint16_t)lo | ((uint16_t)hi << 8);
}

/*
 * With IRQ0 enabled and IF=1: watch the PIT counter itself for `periods` timer periods
 * and report whether IRQ0 actually arrived meanwhile. Some firmware leaves the
 * interrupt path closed (LAPIC LINT0 masked, ...); then the caller falls back to
 * polling instead of a kernel whose clock and scheduler silently never tick.
 */
bool timer_irq_check(uint32_t periods) {
    uint64_t before = global_timer.irq_ticks;
    uint32_t clocks = 0, target = (uint32_t)pit_divisor * periods;
    uint16_t last = pit_latch_count0();
    /* Bounded even if the PIT counter were frozen: ~2^26 latches is well over 1 s. */
    for (uint32_t spins = 0; spins < (1u << 26) && clocks < target; spins++) {
        uint16_t cur = pit_latch_count0();
        clocks += cur <= last ? (uint32_t)(last - cur) : (uint32_t)last + pit_divisor - cur;
        last = cur;
        if (global_timer.irq_ticks - before >= 2u) {
            return true;
        }
    }
    return global_timer.irq_ticks - before >= 2u;
}

bool timer_is_poll_mode(void) {
    return pit_poll_mode;
}

/*
 * Poll-mode clock: one tick per PIT period (pit_divisor input clocks), the same rate
 * as IRQ0. Counter deltas between polls are accumulated in input clocks and whole
 * periods are emitted as ticks. A single poll can only see one reload, so a gap longer
 * than one period between polls under-counts; it never over-counts.
 */
void timer_poll(void) {
    static uint16_t last_count;
    static bool have_last;
    static uint32_t elapsed_clocks;

    if (!pit_poll_mode || pit_divisor == 0) {
        return;
    }

    uint16_t cur = pit_latch_count0();
    if (!have_last) {
        last_count = cur;
        elapsed_clocks = 0;
        have_last = true;
        return;
    }

    /* Mode 2 counts pit_divisor..1 then reloads; each step is one input clock. */
    uint32_t advanced;
    if (cur <= last_count) {
        advanced = (uint32_t)last_count - (uint32_t)cur;
    } else {
        advanced = (uint32_t)last_count + ((uint32_t)pit_divisor - (uint32_t)cur);
    }
    last_count = cur;

    elapsed_clocks += advanced;
    while (elapsed_clocks >= pit_divisor) {
        elapsed_clocks -= pit_divisor;
        global_timer.ticks++;
        if (global_timer.tick_handler) {
            global_timer.tick_handler();
        }
    }
}

// Get current tick count
uint64_t timer_get_ticks(void) {
    return global_timer.ticks;
}

// Get uptime in milliseconds
uint64_t timer_get_uptime_ms(void) {
    if (global_timer.frequency == 0) {
        return 0;
    }
    return (global_timer.ticks * 1000) / global_timer.frequency;
}

// Set custom tick handler
void timer_set_tick_handler(void (*handler)(void)) {
    global_timer.tick_handler = handler;
    
}

// Convert milliseconds to ticks
uint64_t timer_ms_to_ticks(uint64_t ms) {
    return (ms * global_timer.frequency) / 1000;
}
