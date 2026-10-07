#ifndef CONSOLE_H
#define CONSOLE_H

#include <stdbool.h>
#include <stdint.h>

/*
 * Console text UX (this file + console.c).
 * Pixels / CRTC live in the Rust display drive; Rust pops write via console_ffi.
 *
 * Writing surface (prefer these):
 *   console_print / _color / println / println_color / putchar / newline
 *   console_print_error / _success / _info / _warning
 *   console_clear / set_cursor / draw_header / draw_prompt*
 */

/* Legacy VGA text defaults (also used when no GOP). */
#define VGA_WIDTH 80
#define VGA_HEIGHT 25
#define VGA_MEMORY_ADDRESS 0xB8000

/* Full-bleed FB grid at 8×16 (160×45 @720p, 240×67 @1080p). */
#define CONSOLE_MAX_COLS 240
#define CONSOLE_MAX_ROWS 70
#define CONSOLE_MAX_CELL_BYTES (CONSOLE_MAX_COLS * CONSOLE_MAX_ROWS * 2)

/* Runtime grid (FB fills the panel; VGA stays 80×25). */
unsigned int console_cols(void);
unsigned int console_rows(void);

/* Last two rows: heartbeat + status; shell scrolls above them. */
#define CONSOLE_STATUS_ROW (console_rows() - 1u)
#define CONSOLE_HEARTBEAT_ROW (console_rows() - 2u)
#define CONSOLE_SCROLL_ROWS (console_rows() - 2u)

/* Color definitions (VGA text mode) */
#define COLOR_BLACK         0x00
#define COLOR_BLUE          0x01
#define COLOR_GREEN         0x02
#define COLOR_CYAN          0x03
#define COLOR_RED           0x04
#define COLOR_MAGENTA       0x05
#define COLOR_BROWN         0x06
#define COLOR_LIGHT_GRAY    0x07
#define COLOR_DARK_GRAY     0x08
#define COLOR_LIGHT_BLUE    0x09
#define COLOR_LIGHT_GREEN   0x0A
#define COLOR_LIGHT_CYAN    0x0B
#define COLOR_LIGHT_RED     0x0C
#define COLOR_LIGHT_MAGENTA 0x0D
#define COLOR_YELLOW        0x0E
#define COLOR_WHITE         0x0F

#define BG_BLACK        0x00
#define BG_BLUE         0x10
#define BG_GREEN        0x20
#define BG_CYAN         0x30
#define BG_RED          0x40
#define BG_MAGENTA      0x50
#define BG_BROWN        0x60
#define BG_LIGHT_GRAY   0x70
#define BG_DARK_GRAY    0x80
#define BG_LIGHT_BLUE   0x90
#define BG_LIGHT_GREEN  0xA0
#define BG_LIGHT_CYAN   0xB0
#define BG_LIGHT_RED    0xC0
#define BG_LIGHT_MAGENTA 0xD0
#define BG_YELLOW       0xE0
#define BG_WHITE        0xF0

#define CONSOLE_BG_COLOR     BG_BLACK
#define CONSOLE_FG_COLOR     COLOR_WHITE
#define CONSOLE_PROMPT_COLOR COLOR_LIGHT_GREEN
#define CONSOLE_ERROR_COLOR  COLOR_LIGHT_RED
#define CONSOLE_SUCCESS_COLOR COLOR_LIGHT_GREEN
#define CONSOLE_INFO_COLOR   COLOR_LIGHT_CYAN
#define CONSOLE_WARNING_COLOR COLOR_YELLOW
#define CONSOLE_HEADER_COLOR COLOR_LIGHT_MAGENTA

#define SCROLLBACK_LINES 2000
#define SCROLLBACK_LINE_SIZE (CONSOLE_MAX_COLS * 2)

typedef struct {
    unsigned int cursor_x;
    unsigned int cursor_y;
    unsigned char current_color;
    bool cursor_visible;
    bool double_buffer_enabled;
    int scroll_offset;
    bool wrap_enabled;
} ConsoleState;

typedef struct {
    char buffer[SCROLLBACK_LINES * SCROLLBACK_LINE_SIZE];
    unsigned int current_line;
    unsigned int total_lines;
} ScrollbackBuffer;

void console_init(void);
void console_clear(void);

void console_present(void);
void console_sync_begin(void);
void console_sync_end(void);
char* console_get_buffer(void);
bool console_fb_active(void);
void console_fb_paint_background(uint32_t rgb);
void console_fb_relayout(void);
void console_set_color(unsigned char color);
unsigned char console_get_color(void);
void console_set_cursor(unsigned int x, unsigned int y);
void console_get_cursor(unsigned int* x, unsigned int* y);
void console_putchar(char c);
void console_print(const char* str);
void console_print_color(const char* str, unsigned char color);
void console_println(const char* str);
void console_println_color(const char* str, unsigned char color);
void console_scroll(void);
void console_newline(void);
void console_backspace(void);
void console_draw_box(unsigned int x, unsigned int y, unsigned int width, unsigned int height, unsigned char color);
void console_draw_header(const char* title);
void console_draw_prompt(void);
void console_draw_prompt_with_path(const char* path);
void console_print_status_bar(void);
void console_heartbeat_tick(void);
void console_print_error(const char* message);
void console_print_success(const char* message);
void console_print_info(const char* message);
void console_print_warning(const char* message);

unsigned char make_color(unsigned char foreground, unsigned char background);
void console_center_text(const char* text, unsigned int y, unsigned char color);
void console_draw_separator(unsigned int y, unsigned char color);

void console_enable_double_buffer(bool enable);
void console_swap_buffers(void);
void console_flush(void);

void console_scroll_up(void);
void console_scroll_down(void);
void console_scroll_to_bottom(void);
void console_save_line(unsigned int y);
void console_restore_view(void);
void console_set_wrap(bool enabled);
bool console_wrap_enabled(void);

#endif /* CONSOLE_H */
