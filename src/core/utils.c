#include "../includes/utils.h"
#include <stdbool.h>

// Common delay function used across modules
void util_delay(unsigned int milliseconds) {
    for (volatile unsigned int i = 0; i < milliseconds * 1000; i++);
}

// Simple implementation of memset
void *memset(void *s, int c, size_t n) {
    unsigned char *p = s;
    while (n--) {
        *p++ = (unsigned char)c;
    }
    return s;
}

// Simple strlen implementation
size_t strlen_simple(const char *str) {
    size_t len = 0;
    while (str[len] != '\0') {
        len++;
    }
    return len;
}

// Simple strcpy implementation
void strcpy_simple(char *dest, const char *src) {
    size_t i = 0;
    while (src[i] != '\0') {
        dest[i] = src[i];
        i++;
    }
    dest[i] = '\0';
}

// Simple implementation of strcmp
int strcmp(const char *str1, const char *str2) {
    while (*str1 && (*str1 == *str2)) {
        str1++;
        str2++;
    }
    return *(unsigned char *)str1 - *(unsigned char *)str2;
}

// Simple implementation of strncmp
int strncmp(const char *str1, const char *str2, size_t n) {
    size_t i = 0;
    while (i < n && str1[i] && str2[i] && str1[i] == str2[i]) {
        i++;
    }
    if (i == n) {
        return 0;
    }
    return (unsigned char)str1[i] - (unsigned char)str2[i];
}

// Convert integer to string
void int_to_str(int num, char *str) {
    int i = 0;
    bool is_negative = false;
    
    // Handle 0 explicitly
    if (num == 0) {
        str[0] = '0';
        str[1] = '\0';
        return;
    }
    
    // Handle negative numbers
    if (num < 0) {
        is_negative = true;
        num = -num;
    }
    
    // Process individual digits
    while (num != 0) {
        int digit = num % 10;
        str[i++] = digit + '0';
        num = num / 10;
    }
    
    // Add negative sign
    if (is_negative) {
        str[i++] = '-';
    }
    
    str[i] = '\0';
    
    // Reverse the string
    int start = 0;
    int end = i - 1;
    while (start < end) {
        char temp = str[start];
        str[start] = str[end];
        str[end] = temp;
        start++;
        end--;
    }
}

