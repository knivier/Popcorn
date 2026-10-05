#ifndef SHELL_H
#define SHELL_H

#include <stdint.h>

void execute_command(const char *command);
void add_to_history(const char *command);
const char* get_history_command(int offset);
void autocomplete_command(char *buffer, unsigned int *index);
int parse_number(const char* str, uint32_t* result);
void printTerm(const char *str, unsigned char color);

#endif /* SHELL_H */
