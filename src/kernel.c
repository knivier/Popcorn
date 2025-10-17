#include "includes/keyboard_map.h"
#include "includes/pop_module.h"
#include "includes/spinner_pop.h"
#include "includes/console.h"
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
void list_files(void); // Declaration of list_files
bool create_directory(const char* name); // Declaration of create_directory
bool change_directory(const char* name); // Declaration of change_directory
void list_hierarchy(char* vidptr); // Declaration of list_hierarchy

/* current cursor location */
unsigned int current_loc = 0;
/* video memory begins at address 0xb8000 */
char *vidptr = (char*)0xb8000;

// Console state
ConsoleState console_state = {0, 0, CONSOLE_FG_COLOR, true};

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

void kprint(const char *str) {
    console_print(str);
}

void kprint_newline(void) {
    console_newline();
}

void clear_screen(void) {
    console_clear();
}

void printTerm(const char *str, unsigned char color) {
    console_print_color(str, color);
}

/* Simple implementation of memset because we can't use too many external libraries */
void *memset(void *s, int c, size_t n) {
    unsigned char *p = s;
    while (n--) {
        *p++ = (unsigned char)c;
    }
    return s;
}

void keyboard_handler_main(void) {
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
            console_newline();
            console_print_color("Input received: ", CONSOLE_INFO_COLOR);
            console_print_color(input_buffer, CONSOLE_FG_COLOR); // Print the input buffer for debugging
            console_newline();

            execute_command(input_buffer);
            input_index = 0;
            memset(input_buffer, 0, sizeof(input_buffer));
            return;
        }

        if (input_index < sizeof(input_buffer) - 1) {
            input_buffer[input_index++] = keyboard_map[(unsigned char)keycode];
            console_putchar(keyboard_map[(unsigned char)keycode]);
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
void execute_command(const char *command) {
    if (strcmp(command, "help") == 0 || strcmp(command, "halp") == 0) {
        console_println_color("Available Commands:", CONSOLE_HEADER_COLOR);
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
        
        console_print_color("  hang", CONSOLE_PROMPT_COLOR);
        console_println(" - Hangs the system in a loop");
        
        console_print_color("  clear", CONSOLE_PROMPT_COLOR);
        console_println(" - Clears the screen");
        
        console_print_color("  uptime", CONSOLE_PROMPT_COLOR);
        console_println(" - Prints the system uptime");
        
        console_print_color("  halt", CONSOLE_PROMPT_COLOR);
        console_println(" - Halts the system");
        
        console_print_color("  create <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Creates a new file");
        
        console_print_color("  write <filename> <content>", CONSOLE_PROMPT_COLOR);
        console_println(" - Writes content to a file");
        
        console_print_color("  read <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Reads the content of a file");
        
        console_print_color("  delete <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Deletes a file");
        
        console_print_color("  mkdir <dirname>", CONSOLE_PROMPT_COLOR);
        console_println(" - Creates a new directory");
        
        console_print_color("  go <dirname>", CONSOLE_PROMPT_COLOR);
        console_println(" - Changes to the specified directory");
        
        console_print_color("  back", CONSOLE_PROMPT_COLOR);
        console_println(" - Goes back to the previous directory");
        
        console_print_color("  listsys", CONSOLE_PROMPT_COLOR);
        console_println(" - Lists the entire file system hierarchy");
        
        console_print_color("  stop", CONSOLE_PROMPT_COLOR);
        console_println(" - Shuts down the system");
        
        console_newline();
    } else if (strcmp(command, "hang") == 0) {
        console_print_warning("System hanging...");
        spinner_pop_func(current_loc);
        uptime_module.pop_function(current_loc + 16);
        while (1) {
            console_print_color("Hanging...", CONSOLE_ERROR_COLOR);
        }
    } else if (strcmp(command, "clear") == 0) {
        console_clear();
        console_draw_header("Popcorn Kernel v0.4");
        console_print_success("Screen cleared!");
        console_draw_prompt();
    } else if (strcmp(command, "uptime") == 0) {
        console_newline();
        char buffer[64];
        int_to_str(get_tick_count(), buffer);
        console_print_color("Uptime: ", CONSOLE_INFO_COLOR);
        console_print_color(buffer, CONSOLE_FG_COLOR);
        console_println(" ticks");
        
        int ticks = get_tick_count();
        int ticks_per_second = ticks / 150; // Inaccurate estimation, please be aware needs to be tuned!
        int_to_str(ticks_per_second, buffer);
        console_print_color("Estimated seconds: ", CONSOLE_INFO_COLOR);
        console_println_color(buffer, CONSOLE_FG_COLOR);
    } else if (strcmp(command, "halt") == 0) {
        console_print_warning("System halted. Press Enter to continue...");
        while (1) {
            unsigned char status = read_port(KEYBOARD_STATUS_PORT);
            if (status & 0x01) {
                char keycode = read_port(KEYBOARD_DATA_PORT);
                if (keycode == ENTER_KEY_CODE) {
                    console_clear();
                    console_draw_header("Popcorn Kernel v0.4");
                    console_print_success("System resumed!");
                    console_draw_prompt();
                    break;
                }
            }
            halt_module.pop_function(current_loc);
        }
    } else if (strcmp(command, "stop") == 0) {
        console_print_warning("Shutting down...");
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
            console_print_success("File created successfully");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not create file");
        }
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
                console_print_success("File written successfully");
                console_print_color("Filename: ", CONSOLE_INFO_COLOR);
                console_println_color(filename, CONSOLE_FG_COLOR);
            } else {
                console_print_error("Could not write to file");
            }
        } else {
            console_print_error("Invalid command format. Use: write <filename> <content>");
        }
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
            console_print_color("File content: ", CONSOLE_INFO_COLOR);
            console_println_color(content, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not read file");
        }
    } else if (strncmp(command, "delete ", 7) == 0) {
        char filename[21];
        int i = 0;
        while (command[7 + i] != '\0' && i < 20) {
            filename[i] = command[7 + i];
            i++;
        }
        filename[i] = '\0';
        if (delete_file(filename)) {
            console_print_success("File deleted successfully");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not delete file");
        }
    } else if (strncmp(command, "mkdir ", 6) == 0) {
        char dirname[21];
        int i = 0;
        while (command[6 + i] != '\0' && i < 20) {
            dirname[i] = command[6 + i];
            i++;
        }
        dirname[i] = '\0';
        if (create_directory(dirname)) {
            console_print_success("Directory created successfully");
            console_print_color("Directory: ", CONSOLE_INFO_COLOR);
            console_println_color(dirname, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not create directory");
        }
    } else if (strncmp(command, "go ", 3) == 0) {
        char dirname[21];
        int i = 0;
        while (command[3 + i] != '\0' && i < 20) {
            dirname[i] = command[3 + i];
            i++;
        }
        dirname[i] = '\0';
        if (change_directory(dirname)) {
            console_print_success("Changed directory successfully");
            console_print_color("Directory: ", CONSOLE_INFO_COLOR);
            console_println_color(dirname, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not change directory");
        }
    } else if (strcmp(command, "back") == 0) {
        if (change_directory("back")) {
            console_print_success("Changed to parent directory");
        } else {
            console_print_error("Could not change directory");
        }
    } else if (strcmp(command, "listsys") == 0) {
        console_newline();
        console_println_color("File System Hierarchy:", CONSOLE_HEADER_COLOR);
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
        list_hierarchy(vidptr);
        console_newline();
    } else {
        console_print_error("Command not found");
        console_print_color("Command: ", CONSOLE_INFO_COLOR);
        console_println_color(command, CONSOLE_FG_COLOR);
        console_println_color("Type 'help' for available commands", CONSOLE_INFO_COLOR);
    }
    console_newline();
}

/*
Where the magic happens, authored by Knivier
This is executed in kernel.asm and is the heart of the program
It boots the system then initializes all pops, then waits for inputs in a while loop to keep the kernel running
*/
void kmain(void) {
    // Initialize the console system
    console_init();
    
    // Draw the header
    console_draw_header("Popcorn Kernel v0.4");
    
    // Print welcome message
    console_println_color("Welcome to Popcorn Kernel!", CONSOLE_SUCCESS_COLOR);
    console_println_color("A modular metal kernel framework for learning OS development", CONSOLE_INFO_COLOR);
    console_newline();
    
    // Initialize system components
    idt_init();
    kb_init();
    
    // Register pop modules
    register_pop_module(&spinner_module);
    register_pop_module(&uptime_module);
    register_pop_module(&filesystem_module);
    
    // Initialize filesystem
    filesystem_module.pop_function(current_loc);
    
    // Draw initial prompt
    console_draw_prompt();
    
    // Print status bar
    console_print_status_bar();

    // Main input loop
    while (1) {
        // Update status displays
        spinner_pop_func(current_loc);
        uptime_module.pop_function(current_loc + 16);
        
        // Handle keyboard input
        unsigned char status;
        char keycode;
        status = read_port(KEYBOARD_STATUS_PORT);
        if (status & 0x01) {
            keycode = read_port(KEYBOARD_DATA_PORT);
            if (keycode < 0)
                continue;
            if (keycode == ENTER_KEY_CODE) {
                input_buffer[input_index] = '\0';
                console_newline();
                console_print_color("Input received: ", CONSOLE_INFO_COLOR);
                console_print_color(input_buffer, CONSOLE_FG_COLOR);
                console_newline();
                execute_command(input_buffer);
                input_index = 0;
                memset(input_buffer, 0, sizeof(input_buffer));
                console_draw_prompt();
            } else if (input_index < sizeof(input_buffer) - 1) {
                input_buffer[input_index++] = keyboard_map[(unsigned char)keycode];
                console_putchar(keyboard_map[(unsigned char)keycode]);
            }
        }
    }
}