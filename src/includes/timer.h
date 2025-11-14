// src/includes/timer.h
#ifndef TIMER_H
#define TIMER_H

#include <stdint.h>
#include <stdbool.h>
#define PIT_FREQUENCY 1193182  // Base frequency in Hz
#define TIMER_FREQUENCY 100    // Desired timer frequency (100Hz = 10ms intervals)
#define TIMER_INTERRUPT_VECTOR 0x20  // IRQ 0

// Timer state
typedef struct {
    uint64_t ticks;
    uint64_t frequency;
    bool is_active;
    void (*tick_handler)(void);

} TimerState;
extern TimerState global_timer;

// Function declarations
void timer_init(uint32_t frequency_hz);
void timer_interrupt_handler(void);
void timer_enable(void);
void timer_disable(void);
uint64_t timer_get_ticks(void);
uint64_t timer_get_uptime_ms(void);
void timer_set_tick_handler(void (*handler)(void));

// Utility functions
void timer_delay_ms(uint32_t ms);
uint64_t timer_ticks_to_ms(uint64_t ticks);
uint64_t timer_ms_to_ticks(uint64_t ms);

#endif // TIMER_H