// src/includes/dolphin_pop.h
#ifndef DOLPHIN_POP_H
#define DOLPHIN_POP_H

#include "pop_module.h"
#include <stdint.h>
#include <stdbool.h>

// Editor mode
typedef enum {
    EDITOR_MODE_NORMAL,    // Viewing/navigating
    EDITOR_MODE_INSERT,    // Inserting text
    EDITOR_MODE_COMMAND    // Command mode
} EditorMode;

// Text buffer configuration
#define MAX_LINES 100
#define MAX_LINE_LENGTH 80
#define EDITOR_DISPLAY_LINES 20

// Editor state
typedef struct {
    char lines[MAX_LINES][MAX_LINE_LENGTH];
    unsigned int num_lines;
    unsigned int cursor_line;
    unsigned int cursor_col;
    unsigned int scroll_offset;
    EditorMode mode;
    bool modified;
    char filename[64];
    bool active;
} EditorState;

// Editor commands
void dolphin_new(const char* filename);
void dolphin_open(const char* filename);
void dolphin_save(void);
void dolphin_close(void);
void dolphin_help(void);

// Editor operations
void dolphin_insert_char(char ch);
void dolphin_delete_char(void);
void dolphin_new_line(void);
void dolphin_delete_line(void);
void dolphin_move_cursor(int dx, int dy);
void dolphin_render(void);
void dolphin_handle_key(unsigned char keycode);
bool dolphin_is_active(void);

// Pop module function
void dolphin_pop_func(unsigned int start_pos);

// Module definition
extern const PopModule dolphin_module;

// Get editor state
EditorState* dolphin_get_state(void);

#endif // DOLPHIN_POP_H

