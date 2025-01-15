#include "keyboard_map.h"
#include "includes/pop_module.h"
#include "includes/spinner_pop.h"
#include <stddef.h>
#include <stdbool.h>

/* there are 25 lines each of 80 columns; each element takes 2 bytes */
#define LINES 25
#define COLUMNS_IN_LINE 80
#define BYTES_FOR_EACH_ELEMENT 2
#define SCREENSIZE BYTES_FOR_EACH_ELEMENT * COLUMNS_IN_LINE * LINES

#define KEYBOARD_DATA_PORT 0x60
#define KEYBOARD_STATUS_PORT 0x64
#define IDT_SIZE 256
#define INTERRUPT_GATE 0x8e
#define KERNEL_CODE_SEGMENT_OFFSET 0x08

#define ENTER_KEY_CODE 0x1C

extern unsigned char keyboard_map[128];
extern void keyboard_handler(void);
extern char read_port(unsigned short port);
extern void write_port(unsigned short port, unsigned char data);
extern void load_idt(unsigned long *idt_ptr);

/* Global variables */
char input_buffer[128] = {0}; // Increased size to accommodate longer input
unsigned int input_index = 0;

/* Function forward declarations */
void scroll_screen(void);
void execute_command(const char *command);

/* current cursor location */
unsigned int current_loc = 0;
/* video memory begins at address 0xb8000 */
char *vidptr = (char*)0xb8000;

struct IDT_entry {
    unsigned short int offset_lowerbits;
    unsigned short int selector;
    unsigned char zero;
    unsigned char type_attr;
    unsigned short int offset_higherbits;
};

struct IDT_entry IDT[IDT_SIZE];

void idt_init(void)
{
    unsigned long keyboard_address;
    unsigned long idt_address;
    unsigned long idt_ptr[2];

    /* populate IDT entry of keyboard's interrupt */
    keyboard_address = (unsigned long)keyboard_handler;
    IDT[0x21].offset_lowerbits = keyboard_address & 0xffff;
    IDT[0x21].selector = KERNEL_CODE_SEGMENT_OFFSET;
    IDT[0x21].zero = 0;
    IDT[0x21].type_attr = INTERRUPT_GATE;
    IDT[0x21].offset_higherbits = (keyboard_address & 0xffff0000) >> 16;

    /*     Ports
    *    PIC1    PIC2
    *Command 0x20    0xA0
    *Data     0x21    0xA1
    */

    /* ICW1 - begin initialization */
    write_port(0x20 , 0x11);
    write_port(0xA0 , 0x11);

    /* ICW2 - remap offset address of IDT */
    /*
    * In x86 protected mode, we have to remap the PICs beyond 0x20 because
    * Intel has designated the first 32 interrupts as "reserved" for CPU exceptions
    */
    write_port(0x21 , 0x20);
    write_port(0xA1 , 0x28);

    /* ICW3 - setup cascading */
    write_port(0x21 , 0x00);
    write_port(0xA1 , 0x00);

    /* ICW4 - environment info */
    write_port(0x21 , 0x01);
    write_port(0xA1 , 0x01);
    /* Initialization finished */

    /* mask interrupts */
    write_port(0x21 , 0xff);
    write_port(0xA1 , 0xff);

    /* fill the IDT descriptor */
    idt_address = (unsigned long)IDT ;
    idt_ptr[0] = (sizeof (struct IDT_entry) * IDT_SIZE) + ((idt_address & 0xffff) << 16);
    idt_ptr[1] = idt_address >> 16 ;

    load_idt(idt_ptr);
}

void kb_init(void)
{
    /* 0xFD is 11111101 - enables only IRQ1 (keyboard) */
    write_port(0x21 , 0xFD);
}

void kprint(const char *str)
{
    unsigned int i = 0;
    while (str[i] != '\0') {
        vidptr[current_loc++] = str[i++];
        vidptr[current_loc++] = 0x07;
        if (current_loc >= SCREENSIZE) {
            scroll_screen();
        }
    }
}

void kprint_newline(void)
{
    unsigned int line_size = BYTES_FOR_EACH_ELEMENT * COLUMNS_IN_LINE;
    current_loc = current_loc + (line_size - current_loc % (line_size));
    if (current_loc >= SCREENSIZE) {
        scroll_screen();
    }
}

void clear_screen(void)
{
    unsigned int i = 0;
    while (i < SCREENSIZE) {
        vidptr[i++] = ' ';
        vidptr[i++] = 0x07;
    }
    current_loc = 0;
}

void printTerm(const char *str, unsigned char color)
{
    unsigned int i = 0;
    while (str[i] != '\0') {
        vidptr[current_loc++] = str[i++];
        vidptr[current_loc++] = color;
        if (current_loc >= SCREENSIZE) {
            scroll_screen();
        }
    }
}

/* Simple implementation of memset */
void *memset(void *s, int c, size_t n) {
    unsigned char *p = s;
    while (n--) {
        *p++ = (unsigned char)c;
    }
    return s;
}

void keyboard_handler_main(void)
{
    unsigned char status;
    char keycode;

    /* write EOI */
    write_port(0x20, 0x20);

    status = read_port(KEYBOARD_STATUS_PORT);
    /* Lowest bit of status will be set if buffer is not empty */
    if (status & 0x01) {
        keycode = read_port(KEYBOARD_DATA_PORT);
        if (keycode < 0)
            return;

        if (keycode == ENTER_KEY_CODE) {
            input_buffer[input_index] = '\0'; // Null-terminate the input buffer
            kprint("Input received: ");
            kprint(input_buffer); // Print the input buffer for debugging
            kprint("\n");

            execute_command(input_buffer);
            input_index = 0;
            memset(input_buffer, 0, sizeof(input_buffer));
            return;
        }

        if (input_index < sizeof(input_buffer) - 1) {
            input_buffer[input_index++] = keyboard_map[(unsigned char)keycode];
            vidptr[current_loc++] = keyboard_map[(unsigned char)keycode];
            vidptr[current_loc++] = 0x07;
            if (current_loc >= SCREENSIZE) {
                scroll_screen();
            }
        }
    }
}

/* Simple implementation of strcmp */
int strcmp(const char *str1, const char *str2) {
    while (*str1 && (*str1 == *str2)) {
        str1++;
        str2++;
    }
    return *(unsigned char *)str1 - *(unsigned char *)str2;
}

extern const PopModule spinner_module;
extern const PopModule uptime_module;
extern const PopModule shimjapii_module;

void scroll_screen(void)
{
    /* Move the content of the video memory up by one line */
    unsigned int i;
    for (i = 0; i < (LINES - 1) * COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT; i++) {
        vidptr[i] = vidptr[i + COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT];
    }
    /* Clear the last line with the same background color */
    for (i = (LINES - 1) * COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT; i < SCREENSIZE; i += 2) {
        vidptr[i] = ' ';
        vidptr[i + 1] = 0x10; // Dark blue background with white text
    }
    /* Adjust the current location */
    current_loc -= COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT;
}

void execute_command(const char *command)
{
    if (strcmp(command, "help") == 0) {
        kprint("Help Command Executed!");
    } else if (strcmp(command, "hang") == 0) {
        spinner_pop_func(current_loc);
        uptime_module.pop_function(current_loc + 16);
        while (1) {kprint("Hanging..."); }
    } else if (strcmp(command, "cear") == 0) {
        clear_screen();
        kprint("Screen cleared!");
    } else {
        kprint(command);
    }
    kprint_newline();
}

void kmain(void)
{
    const char *boot_msg = "Popcorn v0.3 Popped!!!";

    // Clear the screen with dark blue background
    unsigned int j = 0;
    while (j < 80 * 25 * 2) {
        vidptr[j] = ' ';
        vidptr[j + 1] = 0x10; // Dark blue background
        j += 2;
    }
    
    // Print the welcome message with green text on dark blue background
    unsigned int i = 0;
    while (boot_msg[i] != '\0') {
        vidptr[i * 2] = boot_msg[i];
        vidptr[i * 2 + 1] = 0x12; // Green color on dark blue background
        i++;
    }
    current_loc = i * 2;

    idt_init();
    kb_init();
    register_pop_module(&shimjapii_module);
    register_pop_module(&spinner_module);
    register_pop_module(&uptime_module);

    while (1) {
        /* Wait for user input */
        unsigned char status;
        char keycode;

        execute_all_pops(current_loc);

        status = read_port(KEYBOARD_STATUS_PORT);
        if (status & 0x01) {
            keycode = read_port(KEYBOARD_DATA_PORT);
            if (keycode < 0)
                continue;

            if (keycode == ENTER_KEY_CODE) {
                input_buffer[input_index] = '\0'; // Null-terminate the input buffer
                kprint_newline();
                kprint("Input received: ");
                kprint(input_buffer); // Print the input buffer for debugging
                kprint("\n");

                execute_command(input_buffer);
                input_index = 0;
                memset(input_buffer, 0, sizeof(input_buffer));
            } else if (input_index < sizeof(input_buffer) - 1) {
                input_buffer[input_index++] = keyboard_map[(unsigned char)keycode];
                vidptr[current_loc++] = keyboard_map[(unsigned char)keycode];
                vidptr[current_loc++] = 0x07;
                if (current_loc >= SCREENSIZE) {
                    scroll_screen();
                }
            }
        }
    }
}