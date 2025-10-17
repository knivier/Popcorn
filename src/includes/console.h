#ifndef CONSOLE_H
#define CONSOLE_H

#include <stdbool.h>

// VGA Text Mode Constants
#define VGA_WIDTH 80
#define VGA_HEIGHT 25
#define VGA_MEMORY_SIZE (VGA_WIDTH * VGA_HEIGHT * 2)
#define VGA_MEMORY_ADDRESS 0xB8000

// Color definitions (VGA text mode)
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

// Background colors (shifted left by 4 bits)
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

// Console theme colors
#define CONSOLE_BG_COLOR     BG_BLACK
#define CONSOLE_FG_COLOR     COLOR_LIGHT_GRAY
#define CONSOLE_PROMPT_COLOR COLOR_LIGHT_GREEN
#define CONSOLE_ERROR_COLOR  COLOR_LIGHT_RED
#define CONSOLE_SUCCESS_COLOR COLOR_LIGHT_GREEN
#define CONSOLE_INFO_COLOR   COLOR_LIGHT_CYAN
#define CONSOLE_WARNING_COLOR COLOR_YELLOW
#define CONSOLE_HEADER_COLOR COLOR_LIGHT_MAGENTA

// Console state structure
typedef struct {
    unsigned int cursor_x;
    unsigned int cursor_y;
    unsigned char current_color;
    bool cursor_visible;
} ConsoleState;

// Function declarations
void console_init(void);
void console_clear(void);
void console_set_color(unsigned char color);
void console_set_cursor(unsigned int x, unsigned int y);
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
void console_print_status_bar(void);
void console_print_error(const char* message);
void console_print_success(const char* message);
void console_print_info(const char* message);
void console_print_warning(const char* message);

// Utility functions
unsigned char make_color(unsigned char foreground, unsigned char background);
void console_center_text(const char* text, unsigned int y, unsigned char color);
void console_draw_separator(unsigned int y, unsigned char color);

#endif // CONSOLE_H
