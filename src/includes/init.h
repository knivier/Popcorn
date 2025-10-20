// src/includes/init.h
#ifndef INIT_H
#define INIT_H

#include <stdint.h>
#include <stdbool.h>

// Boot screen states
typedef enum {
    INIT_STATE_START,
    INIT_STATE_MEMORY,
    INIT_STATE_TIMER,
    INIT_STATE_SCHEDULER,
    INIT_STATE_MODULES,
    INIT_STATE_COMPLETE,
    INIT_STATE_WAIT_ENTER
} InitState;

// Boot screen configuration
#define BOOT_SCREEN_WIDTH 80
#define BOOT_SCREEN_HEIGHT 25

// Boot screen colors
#define BOOT_TITLE_COLOR COLOR_LIGHT_MAGENTA
#define BOOT_SUBTITLE_COLOR COLOR_LIGHT_CYAN
#define BOOT_SUCCESS_COLOR COLOR_LIGHT_GREEN
#define BOOT_WARNING_COLOR COLOR_YELLOW
#define BOOT_ERROR_COLOR COLOR_LIGHT_RED
#define BOOT_INFO_COLOR COLOR_LIGHT_GRAY

// Function declarations
void init_boot_screen(void);
void init_draw_header(void);
void init_draw_progress_bar(int current, int total, const char* item);
void init_show_memory_info(void);
void init_show_timer_info(void);
void init_show_scheduler_info(void);
void init_show_modules(void);
void init_wait_for_enter(void);
void init_clear_boot_screen(void);
void init_transition_to_console(void);

// External functions
extern void console_init(void);
extern void console_clear(void);
extern void console_set_cursor(unsigned int x, unsigned int y);
extern void console_print_color(const char* str, unsigned char color);
extern void console_println_color(const char* str, unsigned char color);
extern void console_draw_separator(unsigned int y, unsigned char color);
extern void timer_delay_ms(uint32_t ms);

#endif // INIT_H
