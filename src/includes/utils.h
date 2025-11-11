#ifndef UTILS_H
#define UTILS_H

#include <stddef.h>

// Common utility functions shared across modules

// Delay function for animations and timing
void util_delay(unsigned int milliseconds);

// Memory functions
void* memset(void *s, int c, size_t n);

// String functions
size_t strlen_simple(const char *str);
void strcpy_simple(char *dest, const char *src);
int strcmp(const char *str1, const char *str2);
int strncmp(const char *str1, const char *str2, size_t n);

// Number conversion
void int_to_str(int num, char *str);

#endif // UTILS_H

