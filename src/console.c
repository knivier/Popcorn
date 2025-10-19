#include "includes/console.h"

// External console state (defined in kernel.c)
extern ConsoleState console_state;
static char* vga_memory = (char*)VGA_MEMORY_ADDRESS;

// Double buffering support
static char back_buffer[VGA_MEMORY_SIZE];
static bool buffer_dirty = false;

// Scrollback buffer
static ScrollbackBuffer scrollback = {{0}, 0, 0};

// External variables from kernel.c
extern unsigned int current_loc;
extern char input_buffer[128];
extern unsigned int input_index;

// Initialize the console system
void console_init(void) {
    console_state.cursor_x = 0;
    console_state.cursor_y = 0;
    console_state.current_color = CONSOLE_FG_COLOR;
    console_state.cursor_visible = true;
    console_state.double_buffer_enabled = false;
    console_state.scroll_offset = 0;
    
    // Initialize back buffer
    for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i += 2) {
        back_buffer[i] = ' ';
        back_buffer[i + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    // Initialize scrollback buffer
    for (unsigned int i = 0; i < SCROLLBACK_LINES * SCROLLBACK_LINE_SIZE; i++) {
        scrollback.buffer[i] = 0;
    }
    scrollback.current_line = 0;
    scrollback.total_lines = 0;
    
    console_clear();
}

// Clear the entire screen
void console_clear(void) {
    for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i += 2) {
        vga_memory[i] = ' ';
        vga_memory[i + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    console_state.cursor_x = 0;
    console_state.cursor_y = 0;
    current_loc = 0;
}

// Set the current text color
void console_set_color(unsigned char color) {
    console_state.current_color = color;
}

// Set cursor position
void console_set_cursor(unsigned int x, unsigned int y) {
    if (x >= VGA_WIDTH) x = VGA_WIDTH - 1;
    if (y >= VGA_HEIGHT) y = VGA_HEIGHT - 1;
    
    console_state.cursor_x = x;
    console_state.cursor_y = y;
    current_loc = (y * VGA_WIDTH + x) * 2;
}

// Put a single character at current cursor position
void console_putchar(char c) {
    if (c == '\n') {
        console_newline();
        return;
    }
    
    if (c == '\r') {
        console_state.cursor_x = 0;
        current_loc = console_state.cursor_y * VGA_WIDTH * 2;
        return;
    }
    
    if (c == '\b') {
        console_backspace();
        return;
    }
    
    // Calculate position in VGA memory
    unsigned int pos = (console_state.cursor_y * VGA_WIDTH + console_state.cursor_x) * 2;
    
    // Write character and color to appropriate buffer
    if (console_state.double_buffer_enabled) {
        back_buffer[pos] = c;
        back_buffer[pos + 1] = console_state.current_color;
        buffer_dirty = true;
    } else {
        vga_memory[pos] = c;
        vga_memory[pos + 1] = console_state.current_color;
    }
    
    // Move cursor
    console_state.cursor_x++;
    if (console_state.cursor_x >= VGA_WIDTH) {
        console_newline();
    }
    
    current_loc = (console_state.cursor_y * VGA_WIDTH + console_state.cursor_x) * 2;
}

// Print a string with current color
void console_print(const char* str) {
    while (*str) {
        console_putchar(*str++);
    }
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
    
    if (console_state.cursor_y >= VGA_HEIGHT) {
        console_scroll();
        console_state.cursor_y = VGA_HEIGHT - 1;
    }
    
    current_loc = console_state.cursor_y * VGA_WIDTH * 2;
}

// Scroll the screen up by one line
void console_scroll(void) {
    // Save top line to scrollback before scrolling
    console_save_line(0);
    
    // Move all lines up by one
    for (unsigned int y = 0; y < VGA_HEIGHT - 1; y++) {
        for (unsigned int x = 0; x < VGA_WIDTH; x++) {
            unsigned int src_pos = ((y + 1) * VGA_WIDTH + x) * 2;
            unsigned int dst_pos = (y * VGA_WIDTH + x) * 2;
            
            vga_memory[dst_pos] = vga_memory[src_pos];
            vga_memory[dst_pos + 1] = vga_memory[src_pos + 1];
        }
    }
    
    // Clear the bottom line
    for (unsigned int x = 0; x < VGA_WIDTH; x++) {
        unsigned int pos = ((VGA_HEIGHT - 1) * VGA_WIDTH + x) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    // Reset scroll offset when new content appears
    console_state.scroll_offset = 0;
}

// Handle backspace
void console_backspace(void) {
    if (console_state.cursor_x > 0) {
        console_state.cursor_x--;
        unsigned int pos = (console_state.cursor_y * VGA_WIDTH + console_state.cursor_x) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
        current_loc = pos;
    } else if (console_state.cursor_y > 0) {
        // Move to end of previous line if at start of line
        console_state.cursor_y--;
        console_state.cursor_x = VGA_WIDTH - 1;
        unsigned int pos = (console_state.cursor_y * VGA_WIDTH + console_state.cursor_x) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
        current_loc = pos;
    }
}

// Draw a box with borders
void console_draw_box(unsigned int x, unsigned int y, unsigned int width, unsigned int height, unsigned char color) {
    // Draw top and bottom borders
    for (unsigned int i = x; i < x + width; i++) {
        if (i < VGA_WIDTH) {
            // Top border
            if (y < VGA_HEIGHT) {
                unsigned int pos = (y * VGA_WIDTH + i) * 2;
                vga_memory[pos] = (i == x) ? '+' : (i == x + width - 1) ? '+' : '-';
                vga_memory[pos + 1] = color;
            }
            // Bottom border
            if (y + height - 1 < VGA_HEIGHT) {
                unsigned int pos = ((y + height - 1) * VGA_WIDTH + i) * 2;
                vga_memory[pos] = (i == x) ? '+' : (i == x + width - 1) ? '+' : '-';
                vga_memory[pos + 1] = color;
            }
        }
    }
    
    // Draw left and right borders
    for (unsigned int i = y; i < y + height; i++) {
        if (i < VGA_HEIGHT) {
            // Left border
            if (x < VGA_WIDTH) {
                unsigned int pos = (i * VGA_WIDTH + x) * 2;
                vga_memory[pos] = (i == y) ? '+' : (i == y + height - 1) ? '+' : '|';
                vga_memory[pos + 1] = color;
            }
            // Right border
            if (x + width - 1 < VGA_WIDTH) {
                unsigned int pos = (i * VGA_WIDTH + x + width - 1) * 2;
                vga_memory[pos] = (i == y) ? '+' : (i == y + height - 1) ? '+' : '|';
                vga_memory[pos + 1] = color;
            }
        }
    }
}

// Draw a header with title (using ASCII characters for compatibility)
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

// Print status bar at bottom
void console_print_status_bar(void) {
    // Save current cursor position
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;
    
    console_set_cursor(0, VGA_HEIGHT - 1);
    
    // Clear the line first
    for (unsigned int i = 0; i < VGA_WIDTH; i++) {
        unsigned int pos = ((VGA_HEIGHT - 1) * VGA_WIDTH + i) * 2;
        vga_memory[pos] = ' ';
        vga_memory[pos + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
    }
    
    console_set_cursor(0, VGA_HEIGHT - 1);
    console_print_color("Status: Ready | ", CONSOLE_INFO_COLOR);
    console_print_color("Press 'help' for commands", CONSOLE_SUCCESS_COLOR);
    
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
    
    unsigned int x = (VGA_WIDTH - text_len) / 2;
    console_set_cursor(x, y);
    console_print_color(text, color);
}

// Draw a separator line (using ASCII characters)
void console_draw_separator(unsigned int y, unsigned char color) {
    console_set_cursor(0, y);
    for (unsigned int i = 0; i < VGA_WIDTH; i++) {
        console_print_color("-", color);
    }
    console_newline();
}

// Enable or disable double buffering
void console_enable_double_buffer(bool enable) {
    console_state.double_buffer_enabled = enable;
    if (enable) {
        // Copy current VGA memory to back buffer
        for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i++) {
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
        for (unsigned int i = 0; i < VGA_MEMORY_SIZE; i++) {
            vga_memory[i] = back_buffer[i];
        }
        buffer_dirty = false;
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
    if (y >= VGA_HEIGHT) return;
    
    unsigned int line_offset = (scrollback.current_line % SCROLLBACK_LINES) * SCROLLBACK_LINE_SIZE;
    unsigned int vga_offset = y * VGA_WIDTH * 2;
    
    // Copy line from VGA memory to scrollback
    for (unsigned int i = 0; i < SCROLLBACK_LINE_SIZE; i++) {
        scrollback.buffer[line_offset + i] = vga_memory[vga_offset + i];
    }
    
    scrollback.current_line++;
    if (scrollback.total_lines < SCROLLBACK_LINES) {
        scrollback.total_lines++;
    }
}

// Scroll up in history (Page Up)
void console_scroll_up(void) {
    if (console_state.scroll_offset >= (int)scrollback.total_lines - 1) {
        return;  // Already at top of history
    }
    
    console_state.scroll_offset++;
    console_restore_view();
}

// Scroll down in history (Page Down)
void console_scroll_down(void) {
    if (console_state.scroll_offset <= 0) {
        return;  // Already at current view
    }
    
    console_state.scroll_offset--;
    console_restore_view();
}

// Restore view based on scroll offset
void console_restore_view(void) {
    if (console_state.scroll_offset == 0) {
        // No scrolling, just return to normal view
        return;
    }
    
    // Calculate which lines to show
    for (unsigned int y = 0; y < VGA_HEIGHT; y++) {
        int history_line = (int)scrollback.current_line - console_state.scroll_offset - (VGA_HEIGHT - y);
        
        if (history_line >= 0 && history_line < (int)scrollback.total_lines) {
            unsigned int line_offset = (history_line % SCROLLBACK_LINES) * SCROLLBACK_LINE_SIZE;
            unsigned int vga_offset = y * VGA_WIDTH * 2;
            
            // Copy from scrollback to VGA
            for (unsigned int i = 0; i < SCROLLBACK_LINE_SIZE; i++) {
                vga_memory[vga_offset + i] = scrollback.buffer[line_offset + i];
            }
        } else {
            // Clear line if no history
            unsigned int vga_offset = y * VGA_WIDTH * 2;
            for (unsigned int i = 0; i < VGA_WIDTH; i++) {
                vga_memory[vga_offset + i * 2] = ' ';
                vga_memory[vga_offset + i * 2 + 1] = CONSOLE_BG_COLOR | CONSOLE_FG_COLOR;
            }
        }
    }
}
