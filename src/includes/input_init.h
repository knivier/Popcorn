// src/input_init.h
#ifndef INPUT_INIT_H
#define INPUT_INIT_H

// Keyboard port constants
#define KEYBOARD_DATA_PORT    0x60
#define KEYBOARD_STATUS_PORT  0x64

// Function to initialize keyboard
void init_keyboard(void);

// Function to read from port
unsigned char inb(unsigned short port);

// Function to write to port
void outb(unsigned short port, unsigned char data);

// Callback type for keyboard events
typedef void (*keyboard_callback_t)(unsigned char scancode);

// Register a callback for keyboard events
void register_keyboard_callback(keyboard_callback_t callback);

#endif