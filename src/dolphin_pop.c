// src/dolphin_pop.c - Dolphin Text Editor
#include "includes/dolphin_pop.h"
#include "includes/console.h"
#include <stddef.h>
#include <stdbool.h>

// External filesystem functions
extern bool write_file(const char* name, const char* content);
extern const char* read_file(const char* name);
extern bool delete_file(const char* name);

// External console state
extern ConsoleState console_state;

// External keyboard functions
extern char read_port(unsigned short port);
#define KEYBOARD_STATUS_PORT 0x64
#define KEYBOARD_DATA_PORT 0x60

// Keyboard scancodes
#define KEY_ENTER 0x1C
#define KEY_BACKSPACE 0x0E
#define KEY_ESC 0x01
#define KEY_UP_ARROW 0x48
#define KEY_DOWN_ARROW 0x50
#define KEY_LEFT_ARROW 0x4B
#define KEY_RIGHT_ARROW 0x4D
#define KEY_ESC_RELEASE 0x81

// Editor state
static EditorState editor = {{{0}}, 0, 0, 0, 0, EDITOR_MODE_NORMAL, false, {0}, false};

// Helper: string length
static size_t str_len(const char* str) {
    size_t len = 0;
    while (str[len] != '\0') len++;
    return len;
}

// Helper: string copy with limit
static void str_copy(char* dest, const char* src, size_t max_len) {
    size_t i;
    for (i = 0; i < max_len - 1 && src[i] != '\0'; i++) {
        dest[i] = src[i];
    }
    dest[i] = '\0';
}

// Helper: check if string ends with suffix
static bool ends_with(const char* str, const char* suffix) {
    size_t str_len_val = str_len(str);
    size_t suffix_len = str_len(suffix);
    
    if (suffix_len > str_len_val) return false;
    
    for (size_t i = 0; i < suffix_len; i++) {
        if (str[str_len_val - suffix_len + i] != suffix[i]) {
            return false;
        }
    }
    return true;
}

// Get editor state
EditorState* dolphin_get_state(void) {
    return &editor;
}

// Check if editor is active
bool dolphin_is_active(void) {
    return editor.active;
}

// Helper: Initialize editor UI (header and separator)
static void dolphin_init_ui(void) {
    console_clear();
    extern void console_set_cursor(unsigned int x, unsigned int y);
    console_set_cursor(0, 0);
    console_println_color("=== Dolphin Text Editor ===", CONSOLE_HEADER_COLOR);
    console_print_color("Editing: ", CONSOLE_INFO_COLOR);
    console_println_color(editor.filename, CONSOLE_SUCCESS_COLOR);
    console_draw_separator(2, CONSOLE_FG_COLOR);
}

// Create new file
void dolphin_new(const char* filename) {
    if (!filename || filename[0] == '\0') {
        console_print_error("Usage: dol -new <filename>");
        return;
    }
    
    // Ensure .txt extension
    str_copy(editor.filename, filename, sizeof(editor.filename));
    if (!ends_with(editor.filename, ".txt")) {
        size_t len = str_len(editor.filename);
        if (len < sizeof(editor.filename) - 4) {
            editor.filename[len] = '.';
            editor.filename[len + 1] = 't';
            editor.filename[len + 2] = 'x';
            editor.filename[len + 3] = 't';
            editor.filename[len + 4] = '\0';
        }
    }
    
    // Initialize empty document
    editor.num_lines = 1;
    for (unsigned int i = 0; i < MAX_LINES; i++) {
        editor.lines[i][0] = '\0';
    }
    editor.cursor_line = 0;
    editor.cursor_col = 0;
    editor.scroll_offset = 0;
    editor.mode = EDITOR_MODE_INSERT;
    editor.modified = false;
    editor.active = true;
    
    dolphin_init_ui();
    dolphin_render();
}

// Open existing file
void dolphin_open(const char* filename) {
    if (!filename || filename[0] == '\0') {
        console_print_error("Usage: dol -open <filename>");
        return;
    }
    
    str_copy(editor.filename, filename, sizeof(editor.filename));
    if (!ends_with(editor.filename, ".txt")) {
        size_t len = str_len(editor.filename);
        if (len < sizeof(editor.filename) - 4) {
            editor.filename[len] = '.';
            editor.filename[len + 1] = 't';
            editor.filename[len + 2] = 'x';
            editor.filename[len + 3] = 't';
            editor.filename[len + 4] = '\0';
        }
    }
    
    const char* content = read_file(editor.filename);
    if (!content) {
        console_print_error("File not found. Use 'dol -new' to create it");
        return;
    }
    
    // Parse content into lines
    editor.num_lines = 0;
    unsigned int line_pos = 0;
    for (const char* p = content; *p != '\0' && editor.num_lines < MAX_LINES; p++) {
        if (*p == '\n' || line_pos >= MAX_LINE_LENGTH - 1) {
            editor.lines[editor.num_lines][line_pos] = '\0';
            editor.num_lines++;
            line_pos = 0;
        } else {
            editor.lines[editor.num_lines][line_pos++] = *p;
        }
    }
    
    if (line_pos > 0 || editor.num_lines == 0) {
        editor.lines[editor.num_lines][line_pos] = '\0';
        editor.num_lines++;
    }
    
    editor.cursor_line = 0;
    editor.cursor_col = 0;
    editor.scroll_offset = 0;
    editor.mode = EDITOR_MODE_INSERT;
    editor.modified = false;
    editor.active = true;
    
    dolphin_init_ui();
    dolphin_render();
}

// Save current file
void dolphin_save(void) {
    if (!editor.active) {
        console_print_error("No file open in editor");
        return;
    }
    
    // Check for empty file
    if (editor.num_lines == 0 || (editor.num_lines == 1 && editor.lines[0][0] == '\0')) {
        console_set_cursor(0, 22);
        console_print_color("Saving empty file...", CONSOLE_INFO_COLOR);
    }
    
    // Build content string (limit to 1000 chars for filesystem)
    static char content[1001];  // MAX_FILE_CONTENT_LENGTH from filesystem
    unsigned int pos = 0;
    bool truncated = false;
    
    for (unsigned int i = 0; i < editor.num_lines && pos < 999; i++) {
        for (unsigned int j = 0; editor.lines[i][j] != '\0' && pos < 998; j++) {
            content[pos++] = editor.lines[i][j];
            if (pos >= 998) {
                truncated = true;
                break;
            }
        }
        if (truncated) break;
        if (i < editor.num_lines - 1 && pos < 999) {
            content[pos++] = '\n';
        }
    }
    content[pos] = '\0';
    
    // write_file now handles both creating and writing atomically
    if (write_file(editor.filename, content)) {
        editor.modified = false;
        console_set_cursor(0, 22);
        console_print_color("Saved: ", CONSOLE_SUCCESS_COLOR);
        console_print_color(editor.filename, CONSOLE_FG_COLOR);
        console_print(" (");
        char buf[16];
        extern void int_to_str(int num, char *str);
        int_to_str(pos, buf);
        console_print(buf);
        console_print(" bytes)");
        if (truncated) {
            console_print_color(" [TRUNCATED]", CONSOLE_WARNING_COLOR);
        }
    } else {
        console_set_cursor(0, 22);
        console_print_error("Failed to save (filesystem full, name too long, or content too large)");
    }
}

// Close editor
void dolphin_close(void) {
    if (!editor.active) {
        return;
    }
    
    if (editor.modified) {
        console_print_warning("File has unsaved changes!");
        console_println_color("Use 'dol -save' first or 'dol -quit!' to force quit", CONSOLE_INFO_COLOR);
        return;
    }
    
    editor.active = false;
    
    // Properly restore console
    console_clear();
    console_draw_header("Popcorn Kernel v0.5");
    console_print_success("Dolphin editor closed");
    console_print_color("File: ", CONSOLE_INFO_COLOR);
    console_println_color(editor.filename, CONSOLE_FG_COLOR);
    console_newline();
    
    // Restore prompt
    extern const char* get_current_directory(void);
    extern void console_draw_prompt_with_path(const char* path);
    console_draw_prompt_with_path(get_current_directory());
}

// Show help
void dolphin_help(void) {
    console_newline();
    console_println_color("=== Dolphin Text Editor ===", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    console_println_color("Commands:", CONSOLE_INFO_COLOR);
    console_print_color("  dol -new <file>  ", CONSOLE_PROMPT_COLOR);
    console_println(" - Create new text file");
    console_print_color("  dol -open <file> ", CONSOLE_PROMPT_COLOR);
    console_println(" - Open existing text file");
    console_print_color("  dol -save        ", CONSOLE_PROMPT_COLOR);
    console_println(" - Save current file (from shell)");
    console_print_color("  dol -close       ", CONSOLE_PROMPT_COLOR);
    console_println(" - Close editor (from shell)");
    console_print_color("  dol -quit!       ", CONSOLE_PROMPT_COLOR);
    console_println(" - Force quit without saving");
    console_print_color("  dol -help        ", CONSOLE_PROMPT_COLOR);
    console_println(" - Show this help");
    
    console_newline();
    console_println_color("While editing:", CONSOLE_INFO_COLOR);
    console_println(" • Type normally to insert text");
    console_println(" • Backspace to delete characters");
    console_println(" • Enter to create new line");
    
    console_newline();
    console_println_color("Commands (press ESC then type):", CONSOLE_INFO_COLOR);
    console_println(" • w         - Save file");
    console_println(" • q         - Quit (fails if unsaved)");
    console_println(" • q!        - Force quit without saving");
    console_println(" • wq or x   - Save and quit");
    
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
}

// Insert character at cursor
void dolphin_insert_char(char ch) {
    if (!editor.active || editor.cursor_line >= editor.num_lines) return;
    
    size_t line_len = str_len(editor.lines[editor.cursor_line]);
    
    if (line_len < MAX_LINE_LENGTH - 1 && editor.cursor_col <= line_len) {
        // Shift characters right
        for (size_t i = line_len; i > editor.cursor_col; i--) {
            editor.lines[editor.cursor_line][i] = editor.lines[editor.cursor_line][i - 1];
        }
        editor.lines[editor.cursor_line][editor.cursor_col] = ch;
        editor.lines[editor.cursor_line][line_len + 1] = '\0';
        editor.cursor_col++;
        editor.modified = true;
    }
}

// Delete character at cursor (backspace)
void dolphin_delete_char(void) {
    if (!editor.active || editor.cursor_line >= editor.num_lines) return;
    
    if (editor.cursor_col > 0) {
        // Delete character before cursor
        size_t line_len = str_len(editor.lines[editor.cursor_line]);
        
        // Shift characters left
        for (size_t i = editor.cursor_col - 1; i < line_len; i++) {
            editor.lines[editor.cursor_line][i] = editor.lines[editor.cursor_line][i + 1];
        }
        editor.cursor_col--;
        editor.modified = true;
    } else if (editor.cursor_line > 0) {
        // At start of line - join with previous line
        size_t prev_len = str_len(editor.lines[editor.cursor_line - 1]);
        size_t curr_len = str_len(editor.lines[editor.cursor_line]);
        
        // If combined length fits, merge lines
        if (prev_len + curr_len < MAX_LINE_LENGTH) {
            // Append current line to previous
            for (size_t i = 0; i < curr_len; i++) {
                editor.lines[editor.cursor_line - 1][prev_len + i] = editor.lines[editor.cursor_line][i];
            }
            editor.lines[editor.cursor_line - 1][prev_len + curr_len] = '\0';
            
            // Shift all lines up
            for (unsigned int i = editor.cursor_line; i < editor.num_lines - 1; i++) {
                str_copy(editor.lines[i], editor.lines[i + 1], MAX_LINE_LENGTH);
            }
            
            editor.num_lines--;
            editor.cursor_line--;
            editor.cursor_col = prev_len;
            editor.modified = true;
        }
    }
}

// Create new line
void dolphin_new_line(void) {
    if (!editor.active || editor.num_lines >= MAX_LINES) return;
    
    // Shift lines down
    for (unsigned int i = editor.num_lines; i > editor.cursor_line + 1; i--) {
        str_copy(editor.lines[i], editor.lines[i - 1], MAX_LINE_LENGTH);
    }
    
    // Split current line if needed
    if (editor.cursor_col < str_len(editor.lines[editor.cursor_line])) {
        size_t line_len = str_len(editor.lines[editor.cursor_line]);
        for (size_t i = 0; i < line_len - editor.cursor_col; i++) {
            editor.lines[editor.cursor_line + 1][i] = editor.lines[editor.cursor_line][editor.cursor_col + i];
        }
        editor.lines[editor.cursor_line + 1][line_len - editor.cursor_col] = '\0';
        editor.lines[editor.cursor_line][editor.cursor_col] = '\0';
    } else {
        editor.lines[editor.cursor_line + 1][0] = '\0';
    }
    
    editor.num_lines++;
    editor.cursor_line++;
    editor.cursor_col = 0;
    editor.modified = true;
}

// Render editor content
void dolphin_render(void) {
    if (!editor.active) return;
    
    // Clear screen and redraw
    extern char *vidptr;
    extern void console_set_cursor(unsigned int x, unsigned int y);
    
    // Clear content area (leave top status)
    for (unsigned int y = 4; y < 24; y++) {
        for (unsigned int x = 0; x < 80; x++) {
            vidptr[(y * 80 + x) * 2] = ' ';
            vidptr[(y * 80 + x) * 2 + 1] = 0x07;
        }
    }
    
    // Display lines
    for (unsigned int i = 0; i < EDITOR_DISPLAY_LINES && i < editor.num_lines; i++) {
        unsigned int line_num = editor.scroll_offset + i;
        unsigned int display_y = 4 + i;
        
        if (line_num < editor.num_lines) {
            console_set_cursor(0, display_y);
            
            // Show line number
            char buf[8];
            extern void int_to_str(int num, char *str);
            int_to_str(line_num + 1, buf);
            unsigned int line_num_width = str_len(buf);
            console_print_color(buf, CONSOLE_INFO_COLOR);
            console_print(": ");
            
            // Calculate cursor position prefix length
            unsigned int prefix_len = line_num_width + 2;  // "N: " or "NN: "
            
            // Display line content with cursor
            size_t line_len = str_len(editor.lines[line_num]);
            for (unsigned int j = 0; j <= line_len && j < MAX_LINE_LENGTH; j++) {
                if (line_num == editor.cursor_line && j == editor.cursor_col) {
                    // Draw cursor at current position
                    if (j < line_len) {
                        // Cursor on character - show inverted
                        console_print_color("_", CONSOLE_SUCCESS_COLOR);
                    } else {
                        // Cursor at end of line
                        console_print_color("_", CONSOLE_SUCCESS_COLOR);
                    }
                }
                if (j < line_len) {
                    console_putchar(editor.lines[line_num][j]);
                }
            }
        }
    }
    
    // Status line
    console_set_cursor(0, 24);
    console_print_color("Line ", CONSOLE_INFO_COLOR);
    char buf[16];
    extern void int_to_str(int num, char *str);
    int_to_str(editor.cursor_line + 1, buf);
    console_print_color(buf, CONSOLE_SUCCESS_COLOR);
    console_print("/");
    int_to_str(editor.num_lines, buf);
    console_print_color(buf, CONSOLE_SUCCESS_COLOR);
    console_print(" Col:");
    int_to_str(editor.cursor_col, buf);
    console_print(buf);
    
    if (editor.modified) {
        console_print_color(" [Modified]", CONSOLE_WARNING_COLOR);
    }
    
    console_print(" | ESC for commands (w,q,wq,q!)");
}

// Handle keyboard input in editor mode
void dolphin_handle_key(unsigned char keycode) {
    extern unsigned char keyboard_map[128];
    
    // Arrow key navigation
    if (keycode == KEY_UP_ARROW) {
        if (editor.cursor_line > 0) {
            editor.cursor_line--;
            size_t line_len = str_len(editor.lines[editor.cursor_line]);
            if (editor.cursor_col > line_len) {
                editor.cursor_col = line_len;
            }
            dolphin_render();
        }
        return;
    } else if (keycode == KEY_DOWN_ARROW) {
        if (editor.cursor_line < editor.num_lines - 1) {
            editor.cursor_line++;
            size_t line_len = str_len(editor.lines[editor.cursor_line]);
            if (editor.cursor_col > line_len) {
                editor.cursor_col = line_len;
            }
            dolphin_render();
        }
        return;
    } else if (keycode == KEY_LEFT_ARROW) {
        if (editor.cursor_col > 0) {
            editor.cursor_col--;
            dolphin_render();
        } else if (editor.cursor_line > 0) {
            editor.cursor_line--;
            editor.cursor_col = str_len(editor.lines[editor.cursor_line]);
            dolphin_render();
        }
        return;
    } else if (keycode == KEY_RIGHT_ARROW) {
        size_t line_len = str_len(editor.lines[editor.cursor_line]);
        if (editor.cursor_col < line_len) {
            editor.cursor_col++;
            dolphin_render();
        } else if (editor.cursor_line < editor.num_lines - 1) {
            editor.cursor_line++;
            editor.cursor_col = 0;
            dolphin_render();
        }
        return;
    }
    
    // Special keys
    if (keycode == KEY_ENTER) {
        dolphin_new_line();
        dolphin_render();
    } else if (keycode == KEY_BACKSPACE) {
        dolphin_delete_char();
        dolphin_render();
    } else if (keycode == KEY_ESC) {
        // Clear bottom line for command
        console_set_cursor(0, 23);
        for (int i = 0; i < 80; i++) {
            console_putchar(' ');
        }
        console_set_cursor(0, 23);
        console_print_color(":", CONSOLE_PROMPT_COLOR);
        
        // Mini command loop
        char cmd_buffer[64] = {0};
        unsigned int cmd_index = 0;
        bool in_command = true;
        
        // Wait for ESC key to be released first
        while (true) {
            unsigned char status = read_port(KEYBOARD_STATUS_PORT);
            if (status & 0x01) {
                unsigned char release_key = read_port(KEYBOARD_DATA_PORT);
                if (release_key == KEY_ESC_RELEASE) break;
            }
        }
        
        while (in_command) {
            unsigned char status = read_port(KEYBOARD_STATUS_PORT);
            if (status & 0x01) {
                unsigned char cmd_key = read_port(KEYBOARD_DATA_PORT);
                
                // Ignore key releases (high bit set)
                if (cmd_key & 0x80) continue;
                
                if (cmd_key == KEY_ENTER) {
                    cmd_buffer[cmd_index] = '\0';
                    
                    console_set_cursor(0, 22);
                    console_print("Executing: [");
                    console_print(cmd_buffer);
                    console_print("]");
                    
                    // Parse command
                    if (cmd_buffer[0] == 'q' && cmd_buffer[1] == '\0') {  // q - quit
                        if (!editor.modified) {
                            dolphin_close();
                            return;  // Exit function completely
                        } else {
                            console_set_cursor(0, 21);
                            console_print_warning("Unsaved changes! Use 'q!' to force or 'wq' to save & quit");
                            return;  // Exit without re-rendering
                        }
                    } else if ((cmd_buffer[0] == 'q' && cmd_buffer[1] == '!') || 
                               (cmd_buffer[0] == 'q' && cmd_buffer[1] == 'u' && cmd_buffer[2] == 'i' && cmd_buffer[3] == 't')) {  // q! - force quit
                        editor.active = false;
                        console_clear();
                        console_draw_header("Popcorn Kernel v0.5");
                        console_println_color("Dolphin closed (changes discarded)", CONSOLE_WARNING_COLOR);
                        console_newline();
                        extern const char* get_current_directory(void);
                        extern void console_draw_prompt_with_path(const char* path);
                        console_draw_prompt_with_path(get_current_directory());
                        return;  // Exit completely
                    } else if (cmd_buffer[0] == 'w' && cmd_buffer[1] == '\0') {  // w - write/save
                        dolphin_save();
                        return;  // Exit and re-render will happen on next key
                    } else if ((cmd_buffer[0] == 'w' && cmd_buffer[1] == 'q' && cmd_buffer[2] == '\0') || 
                               (cmd_buffer[0] == 'x' && cmd_buffer[1] == '\0')) {  // wq or x - save and quit
                        dolphin_save();
                        if (!editor.modified) {  // Only quit if save succeeded
                            dolphin_close();
                        }
                        return;  // Exit completely
                    } else if (cmd_buffer[0] == '\0') {  // Empty - just re-render
                        dolphin_render();
                        return;
                    } else {
                        // Unknown command
                        console_set_cursor(0, 21);
                        console_print_error("Unknown cmd. Use: w (save), q (quit), wq (save & quit), q! (force)");
                        return;
                    }
                } else if (cmd_key == KEY_ESC) {
                    dolphin_render();
                    return;  // Exit command mode
                } else if (cmd_key == KEY_BACKSPACE && cmd_index > 0) {
                    cmd_index--;
                    console_backspace();
                } else {
                    char ch = keyboard_map[cmd_key];
                    if (ch != 0 && cmd_index < 63) {
                        cmd_buffer[cmd_index++] = ch;
                        console_putchar(ch);
                    }
                }
            }
        }
    } else {
        // Regular character
        char ch = keyboard_map[keycode];
        if (ch != 0) {
            dolphin_insert_char(ch);
            dolphin_render();
        }
    }
}

// Pop function
void dolphin_pop_func(unsigned int start_pos) {
    (void)start_pos;
    // Dolphin doesn't need continuous updates
}

// Pop module definition
const PopModule dolphin_module = {
    .name = "dolphin",
    .message = "Dolphin text editor",
    .pop_function = dolphin_pop_func
};

