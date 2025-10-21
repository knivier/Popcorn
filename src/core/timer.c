// src/core/timer.c
#include "../includes/timer.h"
#include "../includes/console.h"
#include <stddef.h>

// External port I/O functions
extern void write_port(unsigned short port, unsigned char data);
extern char read_port(unsigned short port);

// Global timer state
TimerState global_timer = {0};

// Initialize PIT (Programmable Interval Timer)
void timer_init(uint32_t frequency_hz) {
    // Calculate divisor for desired frequency
    uint16_t divisor = PIT_FREQUENCY / frequency_hz;
    
    // Configure PIT channel 0 for mode 3 (square wave generator)
    write_port(0x43, 0x36);  // Channel 0, mode 3, binary
    write_port(0x40, divisor & 0xFF);        // Low byte
    write_port(0x40, (divisor >> 8) & 0xFF); // High byte
    
    // Initialize timer state
    global_timer.ticks = 0;
    global_timer.frequency = frequency_hz;
    global_timer.is_active = false;
    global_timer.tick_handler = NULL;
    
    console_println_color("Timer initialized", CONSOLE_SUCCESS_COLOR);
}

// Timer interrupt handler (called from assembly)
void timer_interrupt_handler(void) {
    // Increment tick counter
    global_timer.ticks++;
    
    // Call registered tick handler if present
    if (global_timer.tick_handler) {
        global_timer.tick_handler();
    }
    
    // Send EOI (End of Interrupt) to PIC
    write_port(0x20, 0x20);
}

// Enable timer interrupts
void timer_enable(void) {
    // Unmask timer interrupt (IRQ 0) in PIC
    char mask = read_port(0x21);
    write_port(0x21, mask & ~0x01);
    global_timer.is_active = true;
}

// Disable timer interrupts
void timer_disable(void) {
    // Mask timer interrupt (IRQ 0) in PIC
    char mask = read_port(0x21);
    write_port(0x21, mask | 0x01);
    global_timer.is_active = false;
}

// Get current tick count
uint64_t timer_get_ticks(void) {
    return global_timer.ticks;
}

// Get uptime in milliseconds
uint64_t timer_get_uptime_ms(void) {
    return (global_timer.ticks * 1000) / global_timer.frequency;
}

// Set custom tick handler
void timer_set_tick_handler(void (*handler)(void)) {
    global_timer.tick_handler = handler;
}

// Delay for specified milliseconds
void timer_delay_ms(uint32_t ms) {
    uint64_t start_ticks = global_timer.ticks;
    uint64_t target_ticks = start_ticks + timer_ms_to_ticks(ms);
    
    while (global_timer.ticks < target_ticks) {
        // Busy wait - in a real system this would yield CPU
        __asm__ volatile("pause");
    }
}

// Convert ticks to milliseconds
uint64_t timer_ticks_to_ms(uint64_t ticks) {
    return (ticks * 1000) / global_timer.frequency;
}

// Convert milliseconds to ticks
uint64_t timer_ms_to_ticks(uint64_t ms) {
    return (ms * global_timer.frequency) / 1000;
}
