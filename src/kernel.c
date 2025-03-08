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
void int_to_str(int num, char *str);
int get_tick_count(void);
int strncmp(const char *str1, const char *str2, unsigned int n); // Declaration of strncmp
bool create_file(const char* name); // Declaration of create_file
bool write_file(const char* name, const char* content); // Declaration of write_file
const char* read_file(const char* name); // Declaration of read_file
bool delete_file(const char* name); // Declaration of delete_file

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
    write_port(0x21 , 0xFD);
}

void kprint(const char *str) // prints the string to the screen with scrolling functionality
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

void kprint_newline(void) // prints a newline character to the screen with scrolling functionality
{
    unsigned int line_size = BYTES_FOR_EACH_ELEMENT * COLUMNS_IN_LINE;
    current_loc = current_loc + (line_size - current_loc % (line_size));
    if (current_loc >= SCREENSIZE) {
        scroll_screen();
    }
}

void clear_screen(void) // Clears the output screen (fills with spaces and resets the cursor)
{
    unsigned int i = 0;
    while (i < SCREENSIZE) {
        vidptr[i++] = ' ';
        vidptr[i++] = 0x10; // Dark blue background with white text
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

/* Simple implementation of memset because we can't use too many external libraries */
void *memset(void *s, int c, size_t n) {
    unsigned char *p = s;
    while (n--) {
        *p++ = (unsigned char)c;
    }
    return s;
}

void keyboard_handler_main(void)  // Kinda buggy but a working 4char implementation
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
            kprint_newline();
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

/* Simple implementation of strcmp because we can't use too many external libraries for this source */
int strcmp(const char *str1, const char *str2) {
    while (*str1 && (*str1 == *str2)) {
        str1++;
        str2++;
    }
    return *(unsigned char *)str1 - *(unsigned char *)str2;
}

/* Simple implementation of strncmp because we can't use too many external libraries for this source */
int strncmp(const char *str1, const char *str2, unsigned int n) {
    unsigned int i = 0;
    while (i < n && str1[i] && str2[i] && str1[i] == str2[i]) {
        i++;
    }
    if (i == n) {
        return 0;
    }
    return (unsigned char)str1[i] - (unsigned char)str2[i];
}

extern const PopModule spinner_module;
extern const PopModule uptime_module;
extern const PopModule halt_module;
extern const PopModule filesystem_module;

void scroll_screen(void)
{
    unsigned int i;
    for (i = 0; i < (LINES - 1) * COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT; i++) {
        vidptr[i] = vidptr[i + COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT];
    }
    for (i = (LINES - 1) * COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT; i < SCREENSIZE; i += 2) {
        vidptr[i] = ' ';
        vidptr[i + 1] = 0x10; // Dark blue background with white text
    }
    /* Adjust the current location */
    current_loc -= COLUMNS_IN_LINE * BYTES_FOR_EACH_ELEMENT;
}

/*
@brief Executes specific commands based on the input string that is given as char
@param string buffer to store the string representation of the integer

*/
void execute_command(const char *command)
{ // uses strcmp to compare the command to the string 
    if (strcmp(command, "help") == 0 || strcmp(command, "halp") == 0) {
        kprint_newline();
        kprint("hang T hangs the system in a loop");
        kprint_newline();
        kprint("cear T clears the screen");
        kprint_newline();
        kprint("upte T prints the uptime");
        kprint_newline();
        kprint("halt T halts the system");
        kprint_newline();
        kprint("create <filename> T creates a new file");
        kprint_newline();
        kprint("write <filename> <content> T writes content to a file");
        kprint_newline();
        kprint("read <filename> T reads the content of a file");
        kprint_newline();
        kprint("delete <filename> T deletes a file");
        kprint_newline();
    } else if (strcmp(command, "hang") == 0) { // Hang implementation causes graphics issues with blue flickering due to system not being able to catch up
        kprint_newline(); // There is no escaping a hang
        spinner_pop_func(current_loc);
        uptime_module.pop_function(current_loc + 16);
        while (1) {kprint("Hanging..."); }
    for (int i = 0; i < 5; i++) {
        kprint(".");
        for (volatile int j = 0; j < 10000000; j++); // Simple delay loop
    }
    } else if (strcmp(command, "cear") == 0) {
        clear_screen();
        kprint("Screen cleared!");
    } else if (strcmp(command, "upte") == 0) {
        kprint_newline();
        char buffer[64];
        int_to_str(get_tick_count(), buffer);
        kprint("Uptime: ");
        kprint(buffer);
        kprint_newline();
        int ticks = get_tick_count();
        int ticks_per_second = ticks / 150; // Inaccurate estimation, please be aware needs to be tuned!
        int_to_str(ticks_per_second, buffer);
        kprint(" (");
        kprint(buffer);
        kprint(" seconds EST)");
        kprint_newline(); 
    } else if (strcmp(command, "halt") == 0) {
        kprint_newline();
        kprint("System halted. Press Enter to continue...");
        while (1) {
            unsigned char status = read_port(KEYBOARD_STATUS_PORT); // Waits for the enter key to resume functionality
            if (status & 0x01) {
                char keycode = read_port(KEYBOARD_DATA_PORT);
                if (keycode == ENTER_KEY_CODE) {
                    clear_screen();
                    kprint("System resumed!");
                    break;
                }
            }
            halt_module.pop_function(current_loc);
        }
    } else if (strcmp(command, "stop") == 0) {
            kprint_newline();
            kprint("Shutting down...");
            // Send shutdown command to QEMU/Bochs via port 0x604
            write_port(0x64, 0xFE);  // Send reset command to keyboard controller
            // If shutdown fails, halt the CPU
            asm volatile("hlt");
    } else if (strncmp(command, "create ", 7) == 0) {
        char filename[21];
        int i = 0;
        while (command[7 + i] != '\0' && i < 20) {
            filename[i] = command[7 + i];
            i++;
        }
        filename[i] = '\0';
        if (create_file(filename)) {
            kprint("File created: ");
            kprint(filename);
        } else {
            kprint("Error: Could not create file");
        }
        kprint_newline();
    } else if (strncmp(command, "write ", 6) == 0) {
        char filename[21];
        char content[101];
        int i = 0;
        int j = 0;
        while (command[6 + i] != ' ' && i < 20 && command[6 + i] != '\0') {
            filename[i] = command[6 + i];
            i++;
        }
        filename[i] = '\0';
        if (command[6 + i] == ' ') {
            i++;
            while (command[6 + i + j] != '\0' && j < 100) {
                content[j] = command[6 + i + j];
                j++;
            }
            content[j] = '\0';
            if (write_file(filename, content)) {
                kprint("File written: ");
                kprint(filename);
            } else {
                kprint("Error: Could not write to file");
            }
        } else {
            kprint("Error: Invalid command format");
        }
        kprint_newline();
    } else if (strncmp(command, "read ", 5) == 0) {
        char filename[21];
        int i = 0;
        while (command[5 + i] != '\0' && i < 20) {
            filename[i] = command[5 + i];
            i++;
        }
        filename[i] = '\0';
        const char* content = read_file(filename);
        if (content) {
            kprint("File content: ");
            kprint(content);
        } else {
            kprint("Error: Could not read file");
        }
        kprint_newline();
    } else if (strncmp(command, "delete ", 7) == 0) {
        char filename[21];
        int i = 0;
        while (command[7 + i] != '\0' && i < 20) {
            filename[i] = command[7 + i];
            i++;
        }
        filename[i] = '\0';
        if (delete_file(filename)) {
            kprint("File deleted: ");
            kprint(filename);
        } else {
            kprint("Error: Could not delete file");
        }
        kprint_newline();
    } else {
        kprint(command);
        kprint(" was not found");
        kprint_newline();
    }
    kprint_newline();
}

/*
Where the magic happens, authored by Knivier
This is executed in kernel.asm and is the heart of the program
It boots the system then initializes all pops, then waits for inputs in a while loop to keep the kernel running
*/
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
    register_pop_module(&spinner_module);
    register_pop_module(&uptime_module);
    register_pop_module(&filesystem_module);
    filesystem_module.pop_function(current_loc); // Test initialization of filesystem pop 
    while (1) {
        /* Wait for user input */
        unsigned char status;
        char keycode;
        status = read_port(KEYBOARD_STATUS_PORT);
        if (status & 0x01) {
            keycode = read_port(KEYBOARD_DATA_PORT);
            if (keycode < 0)
                continue;
            if (keycode == ENTER_KEY_CODE) {
                input_buffer[input_index] = '\0'; // Null-terminate the input buffer, very important!
                kprint_newline();
                kprint("Input received: ");
                kprint(input_buffer); // Print the input buffer for debugging
                kprint_newline();
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