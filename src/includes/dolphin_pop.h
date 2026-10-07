/* Dolphin text editor — implemented in Rust (pops/dolphin.rs), C ABI for shell/kernel. */
#ifndef DOLPHIN_POP_H
#define DOLPHIN_POP_H

#include "pop_module.h"
#include <stdbool.h>

void dolphin_new(const char* filename);
void dolphin_open(const char* filename);
void dolphin_save(void);
void dolphin_close(void);
void dolphin_help(void);
void dolphin_force_quit(void);

void dolphin_insert_char(char ch);
void dolphin_render(void);
void dolphin_handle_key(unsigned char keycode);
bool dolphin_is_active(void);

void dolphin_pop_func(unsigned int start_pos);
extern const PopModule dolphin_module;

#endif /* DOLPHIN_POP_H */
