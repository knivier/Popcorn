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
#define BACKSPACE_KEY_CODE 0x0E

extern unsigned char keyboard_map[128];
extern void keyboard_handler(void);
extern char read_port(unsigned short port);
extern void write_port(unsigned short port, unsigned char data);
extern void load_idt(unsigned long *idt_ptr);

/* Global variables */
char input_buffer[128] = {0}; // Increased size to accommodate longer input
unsigned int input_index = 0;

/* Function forward declarations */
void execute_command(const char *command);
void int_to_str(int num, char *str);
int get_tick_count(void);
int strncmp(const char *str1, const char *str2, unsigned int n); // Declaration of strncmp
bool create_file(const char* name); // Declaration of create_file
bool write_file(const char* name, const char* content); // Declaration of write_file
const char* read_file(const char* name); // Declaration of read_file
bool delete_file(const char* name); // Declaration of delete_file
void list_files(void); // Declaration of list_files
void list_files_console(void); // Declaration of list_files_console
bool create_directory(const char* name); // Declaration of create_directory
bool change_directory(const char* name); // Declaration of change_directory
void list_hierarchy(char* vidptr); // Declaration of list_hierarchy
const char* get_current_directory(void); // Declaration of get_current_directory
const char* search_file(const char* name); // Declaration of search_file
bool copy_file(const char* src_name, const char* dest_path); // Declaration of copy_file

/* current cursor location */
unsigned int current_loc = 0;
/* video memory begins at address 0xb8000 */
char *vidptr = (char*)0xb8000;

// Console state
ConsoleState console_state = {0, 0, CONSOLE_FG_COLOR, true, false};

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
    /* write EOI - End of Interrupt */
    write_port(0x20, 0x20);
    
    /* The actual keyboard input is handled in the main loop */
    /* This function just acknowledges the interrupt */
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

/*
@brief Executes specific commands based on the input string that is given as char
@param command Command string to execute

*/
void execute_command(const char *command) {
    // Validate input command is not NULL or empty
    if (command == NULL || command[0] == '\0') {
        return;
    }
    
    if (strcmp(command, "help") == 0 || strcmp(command, "halp") == 0) {
        console_newline();
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
        
        console_print_color("  rm <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Removes a file (alias for delete)");
        
        console_print_color("  mkdir <dirname>", CONSOLE_PROMPT_COLOR);
        console_println(" - Creates a new directory");
        
        console_print_color("  go <dirname>", CONSOLE_PROMPT_COLOR);
        console_println(" - Changes to the specified directory");
        
        console_print_color("  back", CONSOLE_PROMPT_COLOR);
        console_println(" - Goes back to the previous directory");
        
        console_print_color("  ls", CONSOLE_PROMPT_COLOR);
        console_println(" - Lists files and directories in current directory");
        
        console_print_color("  search <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Searches for a file and shows its location");
        
        console_print_color("  cp <filename> <directory>", CONSOLE_PROMPT_COLOR);
        console_println(" - Copies a file to another directory");
        
        console_print_color("  listsys", CONSOLE_PROMPT_COLOR);
        console_println(" - Lists the entire file system hierarchy");
        
        console_print_color("  stop", CONSOLE_PROMPT_COLOR);
        console_println(" - Shuts down the system");
    } else if (strcmp(command, "hang") == 0) {
        console_print_warning("System hanging...");
        spinner_pop_func(current_loc);
        uptime_module.pop_function(current_loc + 16);
        while (1) {
            console_print_color("Hanging...", CONSOLE_ERROR_COLOR);
        }
    } else if (strcmp(command, "clear") == 0) {
        console_clear();
        console_draw_header("Popcorn Kernel v0.5");
        console_print_success("Screen cleared!");
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
                    console_draw_header("Popcorn Kernel v0.5");
                    console_print_success("System resumed!");
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
        // Validate that there is a filename after "create "
        if (command[7] == '\0' || command[7] == ' ') {
            console_print_error("Usage: create <filename>");
            return;
        }
        
        char filename[21];
        int i = 0;
        while (command[7 + i] != '\0' && command[7 + i] != ' ' && i < 20) {
            filename[i] = command[7 + i];
            i++;
        }
        filename[i] = '\0';
        
        // Validate filename is not empty after trimming
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        if (create_file(filename)) {
            console_print_success("File created successfully");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not create file (file may already exist, name too long, or filesystem full)");
        }
    } else if (strncmp(command, "write ", 6) == 0) {
        // Validate that there is content after "write "
        if (command[6] == '\0' || command[6] == ' ') {
            console_print_error("Usage: write <filename> <content>");
            return;
        }
        
        char filename[21];
        char content[101];
        int i = 0;
        int j = 0;
        while (command[6 + i] != ' ' && i < 20 && command[6 + i] != '\0') {
            filename[i] = command[6 + i];
            i++;
        }
        filename[i] = '\0';
        
        // Validate filename is not empty
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        if (command[6 + i] == ' ') {
            i++;
            // Check if content is provided
            if (command[6 + i] == '\0') {
                console_print_error("Content cannot be empty");
                return;
            }
            
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
                console_print_error("Could not write to file (file may not exist, content too long, or invalid filename)");
            }
        } else {
            console_print_error("Invalid command format. Use: write <filename> <content>");
        }
    } else if (strncmp(command, "read ", 5) == 0) {
        // Validate that there is a filename after "read "
        if (command[5] == '\0' || command[5] == ' ') {
            console_print_error("Usage: read <filename>");
            return;
        }
        
        char filename[21];
        int i = 0;
        while (command[5 + i] != '\0' && command[5 + i] != ' ' && i < 20) {
            filename[i] = command[5 + i];
            i++;
        }
        filename[i] = '\0';
        
        // Validate filename is not empty
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        const char* content = read_file(filename);
        if (content) {
            console_print_color("File content: ", CONSOLE_INFO_COLOR);
            console_println_color(content, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not read file (file may not exist or invalid filename)");
        }
    } else if (strncmp(command, "delete ", 7) == 0) {
        // Validate that there is a filename after "delete "
        if (command[7] == '\0' || command[7] == ' ') {
            console_print_error("Usage: delete <filename>");
            return;
        }
        
        char filename[21];
        int i = 0;
        while (command[7 + i] != '\0' && command[7 + i] != ' ' && i < 20) {
            filename[i] = command[7 + i];
            i++;
        }
        filename[i] = '\0';
        
        // Validate filename is not empty
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        if (delete_file(filename)) {
            console_print_success("File deleted successfully");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not delete file (file may not exist or invalid filename)");
        }
    } else if (strncmp(command, "mkdir ", 6) == 0) {
        // Validate that there is a directory name after "mkdir "
        if (command[6] == '\0' || command[6] == ' ') {
            console_print_error("Usage: mkdir <dirname>");
            return;
        }
        
        char dirname[21];
        int i = 0;
        while (command[6 + i] != '\0' && command[6 + i] != ' ' && i < 20) {
            dirname[i] = command[6 + i];
            i++;
        }
        dirname[i] = '\0';
        
        // Validate directory name is not empty
        if (i == 0) {
            console_print_error("Directory name cannot be empty");
            return;
        }
        
        if (create_directory(dirname)) {
            console_print_success("Directory created successfully");
            console_print_color("Directory: ", CONSOLE_INFO_COLOR);
            console_println_color(dirname, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not create directory (directory may already exist, name too long, or filesystem full)");
        }
    } else if (strncmp(command, "go ", 3) == 0) {
        // Validate that there is a directory name after "go "
        if (command[3] == '\0' || command[3] == ' ') {
            console_print_error("Usage: go <dirname>");
            return;
        }
        
        char dirname[21];
        int i = 0;
        while (command[3 + i] != '\0' && command[3 + i] != ' ' && i < 20) {
            dirname[i] = command[3 + i];
            i++;
        }
        dirname[i] = '\0';
        
        // Validate directory name is not empty
        if (i == 0) {
            console_print_error("Directory name cannot be empty");
            return;
        }
        
        // Prevent "go back" - user should use "back" command instead
        if (strcmp(dirname, "back") == 0) {
            console_print_error("Use 'back' command to go to parent directory (not 'go back')");
            return;
        }
        
        if (change_directory(dirname)) {
            console_print_success("Changed directory successfully");
            console_print_color("Directory: ", CONSOLE_INFO_COLOR);
            console_println_color(dirname, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not change directory (directory may not exist or invalid name)");
        }
    } else if (strncmp(command, "rm ", 3) == 0) {
        // rm command - alias for delete with better error handling
        if (command[3] == '\0' || command[3] == ' ') {
            console_print_error("Usage: rm <filename>");
            return;
        }
        
        char filename[21];
        int i = 0;
        while (command[3 + i] != '\0' && command[3 + i] != ' ' && i < 20) {
            filename[i] = command[3 + i];
            i++;
        }
        filename[i] = '\0';
        
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        if (delete_file(filename)) {
            console_print_success("File removed successfully");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        } else {
            console_print_error("File not found or could not be removed");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        }
    } else if (strcmp(command, "back") == 0) {
        if (change_directory("back")) {
            console_print_success("Changed to parent directory");
        } else {
            console_print_error("Already at root directory - cannot go back further");
        }
    } else if (strcmp(command, "ls") == 0) {
        list_files_console();
    } else if (strncmp(command, "search ", 7) == 0) {
        // Search command - find file and return its directory
        if (command[7] == '\0' || command[7] == ' ') {
            console_print_error("Usage: search <filename>");
            return;
        }
        
        char filename[21];
        int i = 0;
        while (command[7 + i] != '\0' && command[7 + i] != ' ' && i < 20) {
            filename[i] = command[7 + i];
            i++;
        }
        filename[i] = '\0';
        
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        const char* file_path = search_file(filename);
        if (file_path) {
            console_print_success("File found!");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
            console_print_color("Location: ", CONSOLE_INFO_COLOR);
            console_println_color(file_path, CONSOLE_SUCCESS_COLOR);
        } else {
            console_print_error("File not found in filesystem");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        }
    } else if (strncmp(command, "cp ", 3) == 0) {
        // Copy command - copy file to another directory
        if (command[3] == '\0' || command[3] == ' ') {
            console_print_error("Usage: cp <filename> <directory>");
            return;
        }
        
        char filename[21];
        char destdir[100]; // MAX_PATH_LENGTH from filesystem
        int i = 0;
        int j = 0;
        
        // Parse filename
        while (command[3 + i] != ' ' && i < 20 && command[3 + i] != '\0') {
            filename[i] = command[3 + i];
            i++;
        }
        filename[i] = '\0';
        
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        // Skip spaces
        while (command[3 + i] == ' ') {
            i++;
        }
        
        // Parse destination directory
        if (command[3 + i] == '\0') {
            console_print_error("Usage: cp <filename> <directory>");
            return;
        }
        
        while (command[3 + i + j] != '\0' && command[3 + i + j] != ' ' && j < 99) {
            destdir[j] = command[3 + i + j];
            j++;
        }
        destdir[j] = '\0';
        
        if (j == 0) {
            console_print_error("Directory cannot be empty");
            return;
        }
        
        if (copy_file(filename, destdir)) {
            console_print_success("File copied successfully");
            console_print_color("From: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
            console_print_color("To: ", CONSOLE_INFO_COLOR);
            console_println_color(destdir, CONSOLE_FG_COLOR);
        } else {
            console_print_error("Could not copy file (file not found, destination doesn't exist, or already exists there)");
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
    console_draw_header("Popcorn Kernel v0.5");
    
    // Print welcome message
    console_println_color("Welcome to Popcorn Kernel!", CONSOLE_SUCCESS_COLOR);
    console_println_color("A modular kernel framework for learning OS development", CONSOLE_INFO_COLOR);
    console_newline();
    
    // Initialize system components
    idt_init();
    kb_init();
    
    // Register pop modules
    register_pop_module(&spinner_module);
    register_pop_module(&uptime_module);
    register_pop_module(&filesystem_module);
    
    // Initialize filesystem (but don't let it move cursor)
    unsigned int save_x = console_state.cursor_x;
    unsigned int save_y = console_state.cursor_y;
    filesystem_module.pop_function(current_loc);
    console_set_cursor(save_x, save_y);
    
    // Draw initial prompt with path
    console_draw_prompt_with_path(get_current_directory());
    
    // Print status bar
    console_print_status_bar();

    // Main input loop
    while (1) {
        // Update status displays (always safe since they save/restore cursor)
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
                execute_command(input_buffer);
                input_index = 0;
                memset(input_buffer, 0, sizeof(input_buffer));
                console_newline();
                console_draw_prompt_with_path(get_current_directory());
            } else if (keycode == BACKSPACE_KEY_CODE) {
                // Handle backspace: remove from buffer and screen
                if (input_index > 0) {
                    input_index--;
                    input_buffer[input_index] = '\0';
                    console_backspace();
                }
            } else if (input_index < sizeof(input_buffer) - 1) {
                char ch = keyboard_map[(unsigned char)keycode];
                if (ch != 0) {  // Only add printable characters
                    input_buffer[input_index++] = ch;
                    console_putchar(ch);
                }
            }
        }
    }
}