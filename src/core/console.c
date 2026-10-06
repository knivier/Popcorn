#include "../includes/console.h"
#include "../includes/multiboot2.h"
#include "../includes/utils.h"
#include "../includes/uefi_input.h"
#include "../includes/timer.h"
#include "../includes/driver_abi.h"
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define HEARTBEAT_ROW CONSOLE_HEARTBEAT_ROW

static volatile uint64_t console_alive_seq;

// External console state (defined in core/kernel.c)
extern ConsoleState console_state;

/*
 * Text UX lives here; pixels / CRTC live in the Rust display drive.
 * vga_memory is either 0xB8000 (VGA) or the Rust cell shadow (framebuffer).
 */
static char* vga_memory = (char*)VGA_MEMORY_ADDRESS;

unsigned int console_cols(void) {
    uint32_t c = rust_screen_cols();
    return c ? (unsigned int)c : VGA_WIDTH;
}

unsigned int console_rows(void) {
    uint32_t r = rust_screen_rows();
    return r ? (unsigned int)r : VGA_HEIGHT;
}

static unsigned int console_cell_bytes(void) {
    return console_cols() * console_rows() * 2u;
}

static unsigned int console_line_bytes(void) {
    return console_cols() * 2u;
}

// Double buffering support
static char back_buffer[CONSOLE_MAX_CELL_BYTES];
static bool buffer_dirty = false;

// Scrollback buffer
static ScrollbackBuffer scrollback = {{0}, 0, 0};

/* Live screen cells saved when leaving the bottom (so ↑/↓ can return cleanly). */
static char live_view[CONSOLE_MAX_CELL_BYTES];
static bool live_view_valid = false;

// External variables from core/kernel.c
extern unsigned int current_loc;

static void update_hardware_cursor(unsigned int x, unsigned int y) {
    rust_screen_set_cursor_visible(console_state.cursor_visible ? 1 : 0);
    rust_screen_set_cursor((uint32_t)x, (uint32_t)y);
}

static void console_capture_live_view(void) {
    unsigned int n = console_cell_bytes();
    vga_memory = rust_screen_cells();
    for (unsigned int i = 0; i < n; i++) {
        live_view[i] = vga_memory[i];
    }
    live_view_valid = true;
}

static void console_restore_live_view(void) {
    if (!live_view_valid) {
        return;
    }
    unsigned int n = console_cell_bytes();
    vga_memory = rust_screen_cells();
    for (unsigned int i = 0; i < n; i++) {
        vga_memory[i] = live_view[i];
    }
    rust_screen_invalidate();
    console_present();
    update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
}

bool console_fb_active(void) {
    return rust_screen_backend() == 2;
}

void console_fb_paint_background(uint32_t rgb) {
    rust_screen_paint_bg(rgb);
}

void console_fb_relayout(void) {
    rust_screen_relayout();
}

char* console_get_buffer(void) {
    return rust_screen_cells();
}

void console_sync_begin(void) {
    rust_screen_sync_begin();
}

void console_sync_end(void) {
    rust_screen_sync_end();
}

void console_present(void) {
    rust_screen_present();
}

/* Hand Multiboot/UEFI framebuffer to Rust, or fall back to VGA text. */
static void console_display_init(void) {
    const FramebufferInfo* fb = multiboot2_get_framebuffer();
    if (fb && fb->present) {
        if (rust_screen_init_fb(fb->addr, fb->pitch, fb->width, fb->height, fb->bpp, fb->type,
                                fb->red_pos, fb->red_size, fb->green_pos, fb->green_size,
                                fb->blue_pos, fb->blue_size) == 1) {
            vga_memory = rust_screen_cells();
            return;
        }
    }
    rust_screen_init_vga();
    vga_memory = rust_screen_cells();
}

// Initialize the console system
void console_init(void) {
    console_state.cursor_x = 0;
    console_state.cursor_y = 0;
    console_state.current_color = CONSOLE_FG_COLOR;
    console_state.cursor_visible = true;
    console_state.double_buffer_enabled = false;
    console_state.scroll_offset = 0;
    console_state.wrap_enabled = true;
    live_view_valid = false;
    
    // Initialize back buffer
    for (unsigned int i = 0; i < CONSOLE_MAX_CELL_BYTES; i += 2) {
        back_buffer[i] = ' ';
        back_buffer[i + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    // Initialize scrollback buffer
    for (unsigned int i = 0; i < SCROLLBACK_LINES * SCROLLBACK_LINE_SIZE; i++) {
        scrollback.buffer[i] = 0;
    }
    scrollback.current_line = 0;
    scrollback.total_lines = 0;

    /* UEFI/Ventoy GRUB2 often supplies VBE (tag 7) instead of framebuffer (tag 8). */
    multiboot2_parse_framebuffer();
    console_display_init();

    /* Clear once; boot UI draws next. No interim splash — it flashed over
     * the real startup text on GOP panels. */
    console_clear();
    console_present();
}

// Clear the entire screen
void console_clear(void) {
    unsigned char attr = (unsigned char)(CONSOLE_BG_COLOR | CONSOLE_FG_COLOR);
    rust_screen_clear(attr);
    vga_memory = rust_screen_cells();
    console_state.cursor_x = 0;
    console_state.cursor_y = 0;
    current_loc = 0;
    update_hardware_cursor(0, 0);
}

// Set the current text color
void console_set_color(unsigned char color) {
    console_state.current_color = color;
}

unsigned char console_get_color(void) {
    return console_state.current_color;
}

// Set cursor position
void console_set_cursor(unsigned int x, unsigned int y) {
    if (x >= console_cols()) x = console_cols() - 1;
    if (y >= console_rows()) y = console_rows() - 1;
    
    console_state.cursor_x = x;
    console_state.cursor_y = y;
    current_loc = (y * console_cols() + x) * 2;
    
    update_hardware_cursor(x, y);
}

void console_get_cursor(unsigned int* x, unsigned int* y) {
    if (x) {
        *x = console_state.cursor_x;
    }
    if (y) {
        *y = console_state.cursor_y;
    }
}

// Put a single character at current cursor position
void console_putchar(char c) {
    if (c == '\n') {
        console_newline();
        return;
    }
    
    if (c == '\r') {
        console_state.cursor_x = 0;
        current_loc = console_state.cursor_y * console_cols() * 2;
        return;
    }
    
    if (c == '\b') {
        console_backspace();
        return;
    }
    
    unsigned int cx = console_state.cursor_x;
    unsigned int cy = console_state.cursor_y;
    unsigned int pos = (cy * console_cols() + cx) * 2;

    /* Always keep navy/black background so typed text stays readable. */
    unsigned char attr =
        (unsigned char)(CONSOLE_BG_COLOR | (console_state.current_color & 0x0Fu));
    if ((attr & 0x0Fu) == 0u) {
        attr = (unsigned char)(CONSOLE_BG_COLOR | CONSOLE_FG_COLOR);
    }
    if (console_state.double_buffer_enabled) {
        back_buffer[pos] = c;
        back_buffer[pos + 1] = (char)attr;
        buffer_dirty = true;
    } else {
        rust_screen_write_cell((uint32_t)cx, (uint32_t)cy, (uint8_t)c, attr);
    }
    
    // Move cursor (soft-wrap at edge when wrap_enabled)
    if (console_state.cursor_x + 1u >= console_cols()) {
        if (console_state.wrap_enabled) {
            console_newline();
        } else {
            console_state.cursor_x = console_cols() - 1u;
            current_loc = (console_state.cursor_y * console_cols() + console_state.cursor_x) * 2;
            update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
        }
        return;
    }
    console_state.cursor_x++;
    
    current_loc = (console_state.cursor_y * console_cols() + console_state.cursor_x) * 2;
    
    update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
}

// Print a string with current color
void console_print(const char* str) {
    rust_screen_sync_begin();
    while (*str) {
        console_putchar(*str++);
    }
    rust_screen_sync_end();
}

// Print a string with specific color
void console_print_color(const char* str, unsigned char color) {
    unsigned char old_color = console_state.current_color;
    console_set_color(color);
    console_print(str);
    console_set_color(old_color);
}

// Print a string and add newline
void console_println(const char* str) {
    console_print(str);
    console_newline();
}

// Print a string with specific color and add newline
void console_println_color(const char* str, unsigned char color) {
    console_print_color(str, color);
    console_newline();
}

// Move to next line
void console_newline(void) {
    console_state.cursor_x = 0;
    console_state.cursor_y++;
    if (console_state.cursor_y >= CONSOLE_SCROLL_ROWS) {
        console_scroll();
        console_state.cursor_y = CONSOLE_SCROLL_ROWS - 1u;
    }
    current_loc = console_state.cursor_y * console_cols() * 2;
    update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
}

// Scroll the screen up by one line (leaves heartbeat + status rows alone)
void console_scroll(void) {
    // Save top line to scrollback before scrolling
    console_save_line(0);
    vga_memory = rust_screen_cells();
    
    for (unsigned int y = 0; y + 1u < CONSOLE_SCROLL_ROWS; y++) {
        for (unsigned int x = 0; x < console_cols(); x++) {
            unsigned int src_pos = ((y + 1) * console_cols() + x) * 2;
            unsigned int dst_pos = (y * console_cols() + x) * 2;
            
            vga_memory[dst_pos] = vga_memory[src_pos];
            vga_memory[dst_pos + 1] = vga_memory[src_pos + 1];
        }
    }
    
    for (unsigned int x = 0; x < console_cols(); x++) {
        unsigned int pos = ((CONSOLE_SCROLL_ROWS - 1u) * console_cols() + x) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    // Reset scroll offset when new content appears
    console_state.scroll_offset = 0;
    live_view_valid = false;
    rust_screen_invalidate();
    rust_screen_sync_begin();
    for (unsigned int y = 0; y < CONSOLE_SCROLL_ROWS; y++) {
        rust_screen_mark_row((uint32_t)y);
    }
    rust_screen_sync_end();
}

// Handle backspace
void console_backspace(void) {
    unsigned char attr = (unsigned char)(CONSOLE_BG_COLOR | CONSOLE_FG_COLOR);
    if (console_state.cursor_x > 0) {
        console_state.cursor_x--;
        unsigned int pos = (console_state.cursor_y * console_cols() + console_state.cursor_x) * 2;
        rust_screen_write_cell(console_state.cursor_x, console_state.cursor_y, ' ', attr);
        current_loc = pos;
    } else if (console_state.cursor_y > 0) {
        // Move to end of previous line if at start of line
        console_state.cursor_y--;
        console_state.cursor_x = console_cols() - 1;
        unsigned int pos = (console_state.cursor_y * console_cols() + console_state.cursor_x) * 2;
        rust_screen_write_cell(console_state.cursor_x, console_state.cursor_y, ' ', attr);
        current_loc = pos;
    }
    
    update_hardware_cursor(console_state.cursor_x, console_state.cursor_y);
}

// Draw a box with borders
void console_draw_box(unsigned int x, unsigned int y, unsigned int width, unsigned int height, unsigned char color) {
    rust_screen_sync_begin();
    // Draw top and bottom borders
    for (unsigned int i = x; i < x + width; i++) {
        if (i < console_cols()) {
            // Top border
            if (y < console_rows()) {
                char ch = (i == x) ? '+' : (i == x + width - 1) ? '+' : '-';
                rust_screen_write_cell(i, y, (uint8_t)ch, color);
            }
            // Bottom border
            if (y + height - 1 < console_rows()) {
                char ch = (i == x) ? '+' : (i == x + width - 1) ? '+' : '-';
                rust_screen_write_cell(i, y + height - 1, (uint8_t)ch, color);
            }
        }
    }
    
    // Draw left and right borders
    for (unsigned int i = y; i < y + height; i++) {
        if (i < console_rows()) {
            // Left border
            if (x < console_cols()) {
                char ch = (i == y) ? '+' : (i == y + height - 1) ? '+' : '|';
                rust_screen_write_cell(x, i, (uint8_t)ch, color);
            }
            // Right border
            if (x + width - 1 < console_cols()) {
                char ch = (i == y) ? '+' : (i == y + height - 1) ? '+' : '|';
                rust_screen_write_cell(x + width - 1, i, (uint8_t)ch, color);
            }
        }
    }
    rust_screen_sync_end();
}

// Draw a header with title
void console_draw_header(const char* title) {
    console_set_cursor(0, 1);
    console_print_color("+------------------------------------------------------------------------------+", CONSOLE_HEADER_COLOR);
    console_newline();
    
    console_set_cursor(0, 2);
    console_print_color("|", CONSOLE_HEADER_COLOR);
    
    // Center the title
    unsigned int title_len = 0;
    while (title[title_len]) title_len++;
    unsigned int padding = (78 - title_len) / 2;
    
    for (unsigned int i = 0; i < padding; i++) {
        console_print_color(" ", CONSOLE_HEADER_COLOR);
    }
    console_print_color(title, CONSOLE_HEADER_COLOR);
    for (unsigned int i = 0; i < 78 - title_len - padding; i++) {
        console_print_color(" ", CONSOLE_HEADER_COLOR);
    }
    console_print_color("|", CONSOLE_HEADER_COLOR);
    console_newline();
    
    console_set_cursor(0, 3);
    console_print_color("+------------------------------------------------------------------------------+", CONSOLE_HEADER_COLOR);
    console_newline();
    console_newline();
}

// Draw command prompt
void console_draw_prompt(void) {
    console_print_color("popcorn@kernel:~$ ", CONSOLE_PROMPT_COLOR);
}

// Draw command prompt with current directory path
void console_draw_prompt_with_path(const char* path) {
    console_print_color("popcorn@kernel:", CONSOLE_PROMPT_COLOR);
    console_print_color(path, CONSOLE_INFO_COLOR);
    console_print_color("$ ", CONSOLE_PROMPT_COLOR);
}

static uint64_t console_read_tsc(void) {
    uint32_t lo;
    uint32_t hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((uint64_t)hi << 32) | lo;
}

static void console_heartbeat_paint(uint64_t alive, uint64_t tsc_lo, uint64_t uefi_polls) {
    char num[24];
    char line[48];
    unsigned int i = 0;
    unsigned int j = 0;

    line[i++] = 'A';
    line[i++] = ':';
    uint64_to_str(alive, num);
    for (j = 0; num[j] && i < sizeof(line) - 20; j++) {
        line[i++] = num[j];
    }
    line[i++] = ' ';
    line[i++] = 'T';
    line[i++] = ':';
    uint64_to_str(tsc_lo & 0xFFFFFu, num);
    for (j = 0; num[j] && i < sizeof(line) - 12; j++) {
        line[i++] = num[j];
    }
    if (uefi_polls > 0) {
        line[i++] = ' ';
        line[i++] = 'u';
        line[i++] = ':';
        uint64_to_str(uefi_polls, num);
        for (j = 0; num[j] && i < sizeof(line) - 1; j++) {
            line[i++] = num[j];
        }
    }
    line[i] = '\0';

    {
        unsigned int hb_col = (console_cols() > 28u) ? (console_cols() - 28u) : 0u;
        for (j = 0; line[j] && (hb_col + j) < console_cols(); j++) {
            unsigned int cx = hb_col + j;
            rust_screen_write_cell(
                (uint32_t)cx, (uint32_t)HEARTBEAT_ROW, (uint8_t)line[j],
                (uint8_t)(CONSOLE_BG_COLOR | CONSOLE_SUCCESS_COLOR));
        }
    }
}

void console_heartbeat_tick(void) {
    console_alive_seq++;
    uint64_t polls = uefi_input_available() ? uefi_input_poll_attempts() : 0;
    uint64_t ticks = timer_get_ticks();
    if (ticks == 0) {
        ticks = console_read_tsc() & 0xFFFFFu;
    }
    console_heartbeat_paint(console_alive_seq, ticks, polls);
}

// Print status bar at bottom
void console_print_status_bar(void) {
    // Save current cursor position
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;
    
    console_set_cursor(0, CONSOLE_STATUS_ROW);
    
    // Clear the line first
    rust_screen_sync_begin();
    for (unsigned int i = 0; i < console_cols(); i++) {
        rust_screen_write_cell((uint32_t)i, (uint32_t)CONSOLE_STATUS_ROW, ' ',
                               (uint8_t)(CONSOLE_BG_COLOR | CONSOLE_FG_COLOR));
    }
    rust_screen_sync_end();
    
    console_set_cursor(0, CONSOLE_STATUS_ROW);
    if (console_state.scroll_offset > 0) {
        console_print_color("SCROLL ^v  hist <>  type=bottom | help", CONSOLE_WARNING_COLOR);
    } else {
        console_print_color("Ready | ^v scroll  <> hist | help", CONSOLE_INFO_COLOR);
    }

    // Restore cursor position
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);
}

// Print error message
void console_print_error(const char* message) {
    console_print_color("ERROR: ", CONSOLE_ERROR_COLOR);
    console_print_color(message, CONSOLE_ERROR_COLOR);
    console_newline();
}

// Print success message
void console_print_success(const char* message) {
    console_print_color("SUCCESS: ", CONSOLE_SUCCESS_COLOR);
    console_print_color(message, CONSOLE_SUCCESS_COLOR);
    console_newline();
}

// Print info message
void console_print_info(const char* message) {
    console_print_color("INFO: ", CONSOLE_INFO_COLOR);
    console_print_color(message, CONSOLE_INFO_COLOR);
    console_newline();
}

// Print warning message
void console_print_warning(const char* message) {
    console_print_color("WARNING: ", CONSOLE_WARNING_COLOR);
    console_print_color(message, CONSOLE_WARNING_COLOR);
    console_newline();
}

// Make a color from foreground and background
unsigned char make_color(unsigned char foreground, unsigned char background) {
    return foreground | background;
}

// Center text on a line
void console_center_text(const char* text, unsigned int y, unsigned char color) {
    unsigned int text_len = 0;
    while (text[text_len]) text_len++;
    
    unsigned int x = (console_cols() - text_len) / 2;
    console_set_cursor(x, y);
    console_print_color(text, color);
}

// Draw a separator line
void console_draw_separator(unsigned int y, unsigned char color) {
    console_set_cursor(0, y);
    for (unsigned int i = 0; i < console_cols(); i++) {
        console_print_color("-", color);
    }
    console_newline();
}

// Enable or disable double buffering
void console_enable_double_buffer(bool enable) {
    console_state.double_buffer_enabled = enable;
    if (enable) {
        // Copy current VGA memory to back buffer
        for (unsigned int i = 0; i < CONSOLE_MAX_CELL_BYTES; i++) {
            back_buffer[i] = vga_memory[i];
        }
    } else {
        // Flush any pending changes
        console_flush();
    }
}

// Swap buffers (copy back buffer to VGA memory)
void console_swap_buffers(void) {
    if (!console_state.double_buffer_enabled) {
        return;
    }
    
    if (buffer_dirty) {
        vga_memory = rust_screen_cells();
        for (unsigned int i = 0; i < CONSOLE_MAX_CELL_BYTES; i++) {
            vga_memory[i] = back_buffer[i];
        }
        buffer_dirty = false;
        rust_screen_invalidate();
        console_present();
    }
}

// Flush back buffer to screen
void console_flush(void) {
    if (console_state.double_buffer_enabled) {
        console_swap_buffers();
    }
}

// Save current line to scrollback buffer
void console_save_line(unsigned int y) {
    if (y >= console_rows()) return;
    
    unsigned int line_offset = (scrollback.current_line % SCROLLBACK_LINES) * SCROLLBACK_LINE_SIZE;
    unsigned int vga_offset = y * console_cols() * 2;
    unsigned int nbytes = console_line_bytes();
    vga_memory = rust_screen_cells();
    
    for (unsigned int i = 0; i < SCROLLBACK_LINE_SIZE; i++) {
        if (i < nbytes) {
            scrollback.buffer[line_offset + i] = vga_memory[vga_offset + i];
        } else if ((i & 1u) == 0u) {
            scrollback.buffer[line_offset + i] = ' ';
        } else {
            scrollback.buffer[line_offset + i] = (char)(CONSOLE_BG_COLOR | CONSOLE_FG_COLOR);
        }
    }
    
    scrollback.current_line++;
    if (scrollback.total_lines < SCROLLBACK_LINES) {
        scrollback.total_lines++;
    }
}

void console_set_wrap(bool enabled) {
    console_state.wrap_enabled = enabled;
}

bool console_wrap_enabled(void) {
    return console_state.wrap_enabled;
}

/*
 * Virtual document = scrollback lines || live scroll-region rows.
 * offset 0 = live bottom; each ↑ increases offset by exactly one line.
 */
static int console_scroll_max_offset(void) {
    return (int)scrollback.total_lines;
}

// Scroll up through saved lines (shell ↑)
void console_scroll_up(void) {
    if (console_scroll_max_offset() <= 0) {
        return;
    }
    if (console_state.scroll_offset == 0) {
        console_capture_live_view();
    }
    if (console_state.scroll_offset >= console_scroll_max_offset()) {
        return;
    }
    console_state.scroll_offset++;
    console_restore_view();
}

// Scroll down toward the live prompt (shell ↓)
void console_scroll_down(void) {
    if (console_state.scroll_offset <= 0) {
        return;
    }
    console_state.scroll_offset--;
    if (console_state.scroll_offset == 0) {
        console_restore_live_view();
        console_print_status_bar();
        return;
    }
    console_restore_view();
}

void console_scroll_to_bottom(void) {
    if (console_state.scroll_offset == 0) {
        return;
    }
    console_state.scroll_offset = 0;
    console_restore_live_view();
    console_print_status_bar();
}

static void console_paint_scroll_row(unsigned int y, const char* line_bytes) {
    unsigned int vga_offset = y * console_cols() * 2;
    unsigned int nbytes = console_line_bytes();
    vga_memory = rust_screen_cells();
    for (unsigned int i = 0; i < nbytes; i++) {
        vga_memory[vga_offset + i] = line_bytes[i];
    }
}

static void console_paint_blank_row(unsigned int y) {
    for (unsigned int i = 0; i < console_cols(); i++) {
        rust_screen_write_cell(i, y, ' ',
                               (uint8_t)(CONSOLE_BG_COLOR | CONSOLE_FG_COLOR));
    }
}

// Paint scrollback window for scroll_offset > 0 (one line step per offset).
void console_restore_view(void) {
    if (console_state.scroll_offset == 0) {
        console_restore_live_view();
        return;
    }
    if (!live_view_valid) {
        console_capture_live_view();
    }

    int max_off = console_scroll_max_offset();
    if (console_state.scroll_offset > max_off) {
        console_state.scroll_offset = max_off;
    }
    if (console_state.scroll_offset <= 0) {
        console_restore_live_view();
        return;
    }

    /*
     * Window end (exclusive) in the virtual stream
     * [scrollback 0 .. total) + [live rows 0 .. SCROLL_ROWS).
     */
    int total = (int)scrollback.total_lines;
    int end = total + (int)CONSOLE_SCROLL_ROWS - console_state.scroll_offset;
    int start = end - (int)CONSOLE_SCROLL_ROWS;

    for (unsigned int y = 0; y < CONSOLE_SCROLL_ROWS; y++) {
        int idx = start + (int)y;
        if (idx < 0) {
            console_paint_blank_row(y);
        } else if (idx < total) {
            /* idx 0 = oldest retained line; map through the ring. */
            unsigned int abs_line =
                (scrollback.total_lines < SCROLLBACK_LINES)
                    ? (unsigned int)idx
                    : (scrollback.current_line - scrollback.total_lines + (unsigned int)idx);
            unsigned int buf_index = abs_line % SCROLLBACK_LINES;
            const char* src = &scrollback.buffer[buf_index * SCROLLBACK_LINE_SIZE];
            console_paint_scroll_row(y, src);
        } else {
            int live_row = idx - total;
            if (live_row >= 0 && live_row < (int)CONSOLE_SCROLL_ROWS) {
                const char* src = &live_view[(unsigned int)live_row * console_line_bytes()];
                console_paint_scroll_row(y, src);
            } else {
                console_paint_blank_row(y);
            }
        }
    }
    rust_screen_invalidate();
    console_present();
    console_print_status_bar();
}
