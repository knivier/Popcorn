#include "../includes/pop_module.h"
#include "../includes/console.h"
#include "../includes/error_codes.h"
#include <stdbool.h>
#include <stddef.h> // Include for NULL

// Forward declaration of strrcmp
int strrcmp(const char* str1, const char* str2);

// Global error tracking
static ErrorCode last_filesystem_error = ERR_SUCCESS;

#define MAX_FILES 100
#define MAX_FILENAME_LENGTH 15
#define MAX_FILE_CONTENT_LENGTH 1000
#define MAX_PATH_LENGTH 100

// Structure to represent a file
typedef struct {
    char name[MAX_FILENAME_LENGTH];
    char content[MAX_FILE_CONTENT_LENGTH];
    char path[MAX_PATH_LENGTH];
    bool in_use;
} File;

// Simple in-memory file system
File file_system[MAX_FILES];
char current_path[MAX_PATH_LENGTH] = "root";

// Function to initialize the file system
void init_filesystem() {
    for (int i = 0; i < MAX_FILES; ++i) {
        file_system[i].in_use = false;
    }
    
    // Create base system files in root directory
    // System info file
    int idx = 0;
    file_system[idx].in_use = true;
    file_system[idx].name[0] = 's'; file_system[idx].name[1] = 'y'; file_system[idx].name[2] = 's';
    file_system[idx].name[3] = 't'; file_system[idx].name[4] = 'e'; file_system[idx].name[5] = 'm';
    file_system[idx].name[6] = '.'; file_system[idx].name[7] = 'i'; file_system[idx].name[8] = 'n';
    file_system[idx].name[9] = 'f'; file_system[idx].name[10] = 'o'; file_system[idx].name[11] = '\0';
    file_system[idx].path[0] = 'r'; file_system[idx].path[1] = 'o'; file_system[idx].path[2] = 'o';
    file_system[idx].path[3] = 't'; file_system[idx].path[4] = '\0';
    const char* sys_content = "Popcorn Kernel v0.5 - A modular kernel framework";
    int j = 0;
    while (sys_content[j] != '\0' && j < MAX_FILE_CONTENT_LENGTH - 1) {
        file_system[idx].content[j] = sys_content[j];
        j++;
    }
    file_system[idx].content[j] = '\0';
    
    // README file
    idx = 1;
    file_system[idx].in_use = true;
    file_system[idx].name[0] = 'R'; file_system[idx].name[1] = 'E'; file_system[idx].name[2] = 'A';
    file_system[idx].name[3] = 'D'; file_system[idx].name[4] = 'M'; file_system[idx].name[5] = 'E';
    file_system[idx].name[6] = '.'; file_system[idx].name[7] = 't'; file_system[idx].name[8] = 'x';
    file_system[idx].name[9] = 't'; file_system[idx].name[10] = '\0';
    file_system[idx].path[0] = 'r'; file_system[idx].path[1] = 'o'; file_system[idx].path[2] = 'o';
    file_system[idx].path[3] = 't'; file_system[idx].path[4] = '\0';
    const char* readme_content = "Welcome to Popcorn! Type 'help' for available commands. Use 'ls' to list files.";
    j = 0;
    while (readme_content[j] != '\0' && j < MAX_FILE_CONTENT_LENGTH - 1) {
        file_system[idx].content[j] = readme_content[j];
        j++;
    }
    file_system[idx].content[j] = '\0';
    
    // Create base system directories
    // bin directory
    idx = 2;
    file_system[idx].in_use = true;
    file_system[idx].name[0] = 'b'; file_system[idx].name[1] = 'i'; file_system[idx].name[2] = 'n';
    file_system[idx].name[3] = '\0';
    file_system[idx].path[0] = 'r'; file_system[idx].path[1] = 'o'; file_system[idx].path[2] = 'o';
    file_system[idx].path[3] = 't'; file_system[idx].path[4] = '\0';
    file_system[idx].content[0] = '\0';
    
    // usr directory
    idx = 3;
    file_system[idx].in_use = true;
    file_system[idx].name[0] = 'u'; file_system[idx].name[1] = 's'; file_system[idx].name[2] = 'r';
    file_system[idx].name[3] = '\0';
    file_system[idx].path[0] = 'r'; file_system[idx].path[1] = 'o'; file_system[idx].path[2] = 'o';
    file_system[idx].path[3] = 't'; file_system[idx].path[4] = '\0';
    file_system[idx].content[0] = '\0';
    
    // home directory
    idx = 4;
    file_system[idx].in_use = true;
    file_system[idx].name[0] = 'h'; file_system[idx].name[1] = 'o'; file_system[idx].name[2] = 'm';
    file_system[idx].name[3] = 'e'; file_system[idx].name[4] = '\0';
    file_system[idx].path[0] = 'r'; file_system[idx].path[1] = 'o'; file_system[idx].path[2] = 'o';
    file_system[idx].path[3] = 't'; file_system[idx].path[4] = '\0';
    file_system[idx].content[0] = '\0';
    
    // Add welcome file in home
    idx = 5;
    file_system[idx].in_use = true;
    file_system[idx].name[0] = 'w'; file_system[idx].name[1] = 'e'; file_system[idx].name[2] = 'l';
    file_system[idx].name[3] = 'c'; file_system[idx].name[4] = 'o'; file_system[idx].name[5] = 'm';
    file_system[idx].name[6] = 'e'; file_system[idx].name[7] = '.'; file_system[idx].name[8] = 't';
    file_system[idx].name[9] = 'x'; file_system[idx].name[10] = 't'; file_system[idx].name[11] = '\0';
    file_system[idx].path[0] = 'r'; file_system[idx].path[1] = 'o'; file_system[idx].path[2] = 'o';
    file_system[idx].path[3] = 't'; file_system[idx].path[4] = '|'; file_system[idx].path[5] = 'h';
    file_system[idx].path[6] = 'o'; file_system[idx].path[7] = 'm'; file_system[idx].path[8] = 'e';
    file_system[idx].path[9] = '\0';
    const char* welcome_content = "Welcome to your home directory! This is where you can store your files.";
    j = 0;
    while (welcome_content[j] != '\0' && j < MAX_FILE_CONTENT_LENGTH - 1) {
        file_system[idx].content[j] = welcome_content[j];
        j++;
    }
    file_system[idx].content[j] = '\0';
}

// Function to create a file
// Made static so only internal filesystem functions can create files
// External creation should go through Dolphin editor for .txt files
static bool create_file(const char* name) {
    // Validate input
    if (name == NULL || name[0] == '\0') {
        last_filesystem_error = ERR_INVALID_INPUT;
        return false;
    }
    
    // Check filename length
    int name_len = 0;
    while (name[name_len] != '\0') {
        name_len++;
        if (name_len >= MAX_FILENAME_LENGTH) {
            last_filesystem_error = ERR_NAME_TOO_LONG;
            return false; // Filename too long
        }
    }
    
    // Check for duplicate files in current directory
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use && strrcmp(file_system[i].path, current_path) == 0) {
            if (strrcmp(file_system[i].name, name) == 0) {
                last_filesystem_error = ERR_ALREADY_EXISTS;
                return false; // File already exists
            }
        }
    }
    
    for (int i = 0; i < MAX_FILES; ++i) {
        if (!file_system[i].in_use) {
            int j = 0;
            while (name[j] != '\0' && j < MAX_FILENAME_LENGTH - 1) {
                file_system[i].name[j] = name[j];
                j++;
            }
            file_system[i].name[j] = '\0';

            j = 0;
            while (current_path[j] != '\0' && j < MAX_PATH_LENGTH - 1) {
                file_system[i].path[j] = current_path[j];
                j++;
            }
            file_system[i].path[j] = '\0';

            // Initialize content to empty
            file_system[i].content[0] = '\0';
            file_system[i].in_use = true;
            last_filesystem_error = ERR_SUCCESS;
            return true;
        }
    }
    last_filesystem_error = ERR_NO_SPACE;
    return false; // File system full
}

// Function to write to a file
bool write_file(const char* name, const char* content) {
    // Validate input
    if (name == NULL || name[0] == '\0' || content == NULL) {
        last_filesystem_error = ERR_INVALID_INPUT;
        return false;
    }
    
    // Check content length
    int content_len = 0;
    while (content[content_len] != '\0') {
        content_len++;
        if (content_len >= MAX_FILE_CONTENT_LENGTH) {
            last_filesystem_error = ERR_BUFFER_OVERFLOW;
            return false; // Content too long
        }
    }
    
    // First, try to find and update existing file
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            int j = 0;
            while (file_system[i].name[j] == name[j] && file_system[i].name[j] != '\0' && name[j] != '\0') {
                j++;
            }
            if (file_system[i].name[j] == '\0' && name[j] == '\0' && strrcmp(file_system[i].path, current_path) == 0) {
                j = 0;
                int k = 0;
                while (content[k] != '\0' && j < MAX_FILE_CONTENT_LENGTH - 1) {
                    file_system[i].content[j] = content[k];
                    j++;
                    k++;
                }
                file_system[i].content[j] = '\0';
                last_filesystem_error = ERR_SUCCESS;
                return true;
            }
        }
    }
    
    // File doesn't exist, create it then write to it
    if (create_file(name)) {
        // Recursively call write_file now that the file exists
        return write_file(name, content);
    }
    
    last_filesystem_error = ERR_NO_SPACE;
    return false; // Could not create file
}

// Function to read a file
const char* read_file(const char* name) {
    // Validate input
    if (name == NULL || name[0] == '\0') {
        last_filesystem_error = ERR_INVALID_INPUT;
        return NULL;
    }
    
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            int j = 0;
            while (file_system[i].name[j] == name[j] && file_system[i].name[j] != '\0' && name[j] != '\0') {
                j++;
            }
            if (file_system[i].name[j] == '\0' && name[j] == '\0' && strrcmp(file_system[i].path, current_path) == 0) {
                last_filesystem_error = ERR_SUCCESS;
                return file_system[i].content;
            }
        }
    }
    last_filesystem_error = ERR_NOT_FOUND;
    return NULL; // File not found
}

// Function to calculate the length of a string
size_t strrlen(const char* str) {
    size_t length = 0;
    while (str[length] != '\0') {
        length++;
    }
    return length;
}

// Function to compare two strings up to a given number of characters
int strrncmp(const char* str1, const char* str2, size_t num) {
    for (size_t i = 0; i < num; i++) {
        if (str1[i] != str2[i]) {
            return (unsigned char)str1[i] - (unsigned char)str2[i];
        }
        if (str1[i] == '\0') {
            return 0;
        }
    }
    return 0;
}

// Forward declaration of external int_to_str from uptime_pop.c
extern void int_to_str(int value, char* buffer);

// Function to list all files in the current directory
void list_files_console(void) {
    extern ConsoleState console_state;
    int file_count = 0;
    
    console_newline();
    console_println_color("Files and Directories:", CONSOLE_HEADER_COLOR);
    console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    
    for (int i = 0; i < MAX_FILES; ++i) {
        // Check if the file or directory is in the current path
        if (file_system[i].in_use && strrcmp(file_system[i].path, current_path) == 0) {
            file_count++;
            console_print_color("  ", CONSOLE_FG_COLOR);
            console_println_color(file_system[i].name, CONSOLE_INFO_COLOR);
        }
    }
    
    if (file_count == 0) {
        console_println_color("  (empty directory)", CONSOLE_WARNING_COLOR);
    } else {
        char count_str[32];
        int_to_str(file_count, count_str);
        console_newline();
        console_print_color("Total: ", CONSOLE_INFO_COLOR);
        console_print_color(count_str, CONSOLE_FG_COLOR);
        console_println(" items");
    }
}

// Function to list all files in the current directory (VGA version)
void list_files() {
    char* vidptr = (char*)0xb8000;
    unsigned int pos = 0;

    for (int i = 0; i < MAX_FILES; ++i) {
        // Check if the file or directory is in the current path
        if (file_system[i].in_use && strrncmp(file_system[i].path, current_path, strrlen(current_path)) == 0) {
            // Ensure we are not listing files from a subdirectory
            if (file_system[i].path[strrlen(current_path)] == '\0' || file_system[i].path[strrlen(current_path)] == '|') {
                unsigned int j = 0;
                while (file_system[i].name[j] != '\0') {
                    vidptr[pos] = file_system[i].name[j];
                    vidptr[pos + 1] = 0x07;  // Light grey color
                    ++j;
                    pos += 2;
                }
                vidptr[pos] = ' ';
                vidptr[pos + 1] = 0x07; // Light grey color
                pos += 2;
            }
        }
    }
}

// Function to delete a file
bool delete_file(const char* name) {
    // Validate input
    if (name == NULL || name[0] == '\0') {
        last_filesystem_error = ERR_INVALID_INPUT;
        return false;
    }
    
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            int j = 0;
            while (file_system[i].name[j] == name[j] && file_system[i].name[j] != '\0' && name[j] != '\0') {
                j++;
            }
            if (file_system[i].name[j] == '\0' && name[j] == '\0' && strrcmp(file_system[i].path, current_path) == 0) {
                file_system[i].in_use = false;
                // Clear the file data
                file_system[i].name[0] = '\0';
                file_system[i].content[0] = '\0';
                file_system[i].path[0] = '\0';
                last_filesystem_error = ERR_SUCCESS;
                return true;
            }
        }
    }
    last_filesystem_error = ERR_NOT_FOUND;
    return false; // File not found
}

// Function to create a new directory
bool create_directory(const char* name) {
    // Validate input
    if (name == NULL || name[0] == '\0') {
        last_filesystem_error = ERR_INVALID_INPUT;
        return false;
    }
    
    // Check directory name length
    int name_len = 0;
    while (name[name_len] != '\0') {
        name_len++;
        if (name_len >= MAX_FILENAME_LENGTH) {
            last_filesystem_error = ERR_NAME_TOO_LONG;
            return false; // Directory name too long
        }
    }
    
    // Check if we have room for path
    int current_path_len = 0;
    while (current_path[current_path_len] != '\0') {
        current_path_len++;
    }
    
    // Check if we have room for separator and name
    if (current_path_len >= MAX_PATH_LENGTH - name_len - 2) {
        last_filesystem_error = ERR_BUFFER_OVERFLOW;
        return false; // Path would be too long
    }

    // Check if directory already exists
    for (int k = 0; k < MAX_FILES; ++k) {
        if (file_system[k].in_use && strrcmp(file_system[k].path, current_path) == 0 && strrcmp(file_system[k].name, name) == 0) {
            last_filesystem_error = ERR_ALREADY_EXISTS;
            return false; // Directory already exists
        }
    }

    return create_file(name); // Use create_file to create the directory entry
}

// Function to compare two strings
int strrcmp(const char* str1, const char* str2) {
    while (*str1 && (*str1 == *str2)) {
        str1++;
        str2++;
    }
    return *(unsigned char*)str1 - *(unsigned char*)str2;
}

// Function to change the current directory
bool change_directory(const char* name) {
    // Validate input
    if (name == NULL || name[0] == '\0') {
        last_filesystem_error = ERR_INVALID_INPUT;
        return false;
    }
    
    if (strrcmp(name, "back") == 0) {
        if (strrcmp(current_path, "root") == 0) {
            last_filesystem_error = ERR_INVALID_OPERATION;
            return false; // Already at root, cannot go back further
        }
        int i = 0;
        while (current_path[i] != '\0') {
            i++;
        }
        while (i > 0 && current_path[i] != '|') {
            i--;
        }
        if (i > 0) {
            current_path[i] = '\0';
        } else {
            current_path[0] = 'r';
            current_path[1] = 'o';
            current_path[2] = 'o';
            current_path[3] = 't';
            current_path[4] = '\0';
        }
        last_filesystem_error = ERR_SUCCESS;
        return true;
    } else {
        char new_path[MAX_PATH_LENGTH];
        int i = 0;
        while (current_path[i] != '\0' && i < MAX_PATH_LENGTH - 2) {
            new_path[i] = current_path[i];
            i++;
        }
        
        // Check directory name length
        int name_len = 0;
        while (name[name_len] != '\0') {
            name_len++;
            if (name_len >= MAX_FILENAME_LENGTH) {
                last_filesystem_error = ERR_NAME_TOO_LONG;
                return false; // Directory name too long
            }
        }
        
        // Check if we have room for separator and name
        if (i >= MAX_PATH_LENGTH - name_len - 2) {
            last_filesystem_error = ERR_BUFFER_OVERFLOW;
            return false; // Path would be too long
        }
        
        new_path[i] = '|';
        i++;
        int j = 0;
        while (name[j] != '\0' && i < MAX_PATH_LENGTH - 1) {
            new_path[i] = name[j];
            i++;
            j++;
        }
        new_path[i] = '\0';

        for (int k = 0; k < MAX_FILES; ++k) {
            if (file_system[k].in_use && strrcmp(file_system[k].path, current_path) == 0 && strrcmp(file_system[k].name, name) == 0) {
                int l = 0;
                while (new_path[l] != '\0' && l < MAX_PATH_LENGTH - 1) {
                    current_path[l] = new_path[l];
                    l++;
                }
                current_path[l] = '\0';
                last_filesystem_error = ERR_SUCCESS;
                return true;
            }
        }
        last_filesystem_error = ERR_NOT_FOUND;
        return false; // Directory does not exist
    }
}

// Function to get the last filesystem error
ErrorCode get_last_filesystem_error(void) {
    return last_filesystem_error;
}

// Function to get the current directory path
const char* get_current_directory(void) {
    return current_path;
}

// Function to search for a file and return its directory path
const char* search_file(const char* name) {
    // Validate input
    if (name == NULL || name[0] == '\0') {
        last_filesystem_error = ERR_INVALID_INPUT;
        return NULL;
    }
    
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            // Check if filename matches
            if (strrcmp(file_system[i].name, name) == 0) {
                last_filesystem_error = ERR_SUCCESS;
                return file_system[i].path;
            }
        }
    }
    
    last_filesystem_error = ERR_NOT_FOUND;
    return NULL; // File not found
}

// Function to copy a file to a different directory
bool copy_file(const char* src_name, const char* dest_path) {
    // Validate input
    if (src_name == NULL || src_name[0] == '\0' || dest_path == NULL || dest_path[0] == '\0') {
        last_filesystem_error = ERR_INVALID_INPUT;
        return false;
    }
    
    // Find source file in current directory
    int src_index = -1;
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use && strrcmp(file_system[i].path, current_path) == 0) {
            if (strrcmp(file_system[i].name, src_name) == 0) {
                src_index = i;
                break;
            }
        }
    }
    
    if (src_index == -1) {
        last_filesystem_error = ERR_NOT_FOUND;
        return false; // Source file not found
    }
    
    // Check if destination directory exists
    bool dest_exists = false;
    if (strrcmp(dest_path, "root") == 0) {
        dest_exists = true;
    } else {
        for (int i = 0; i < MAX_FILES; ++i) {
            if (file_system[i].in_use) {
                // Build the full path for comparison
                char check_path[MAX_PATH_LENGTH];
                int k = 0;
                int l = 0;
                while (file_system[i].path[l] != '\0' && k < MAX_PATH_LENGTH - 1) {
                    check_path[k++] = file_system[i].path[l++];
                }
                if (k < MAX_PATH_LENGTH - 1) {
                    check_path[k++] = '|';
                }
                l = 0;
                while (file_system[i].name[l] != '\0' && k < MAX_PATH_LENGTH - 1) {
                    check_path[k++] = file_system[i].name[l++];
                }
                check_path[k] = '\0';
                
                if (strrcmp(check_path, dest_path) == 0) {
                    dest_exists = true;
                    break;
                }
            }
        }
    }
    
    if (!dest_exists) {
        last_filesystem_error = ERR_NOT_FOUND;
        return false; // Destination directory doesn't exist
    }
    
    // Check if file already exists in destination
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use && strrcmp(file_system[i].path, dest_path) == 0) {
            if (strrcmp(file_system[i].name, src_name) == 0) {
                last_filesystem_error = ERR_ALREADY_EXISTS;
                return false; // File already exists in destination
            }
        }
    }
    
    // Find empty slot for the copy
    for (int i = 0; i < MAX_FILES; ++i) {
        if (!file_system[i].in_use) {
            // Copy the file
            int j = 0;
            while (file_system[src_index].name[j] != '\0' && j < MAX_FILENAME_LENGTH - 1) {
                file_system[i].name[j] = file_system[src_index].name[j];
                j++;
            }
            file_system[i].name[j] = '\0';
            
            j = 0;
            while (dest_path[j] != '\0' && j < MAX_PATH_LENGTH - 1) {
                file_system[i].path[j] = dest_path[j];
                j++;
            }
            file_system[i].path[j] = '\0';
            
            j = 0;
            while (file_system[src_index].content[j] != '\0' && j < MAX_FILE_CONTENT_LENGTH - 1) {
                file_system[i].content[j] = file_system[src_index].content[j];
                j++;
            }
            file_system[i].content[j] = '\0';
            
            file_system[i].in_use = true;
            last_filesystem_error = ERR_SUCCESS;
            return true;
        }
    }
    
    last_filesystem_error = ERR_NO_SPACE;
    return false; // Filesystem full
}

// Function to list the entire file system hierarchy
void list_hierarchy(char* vidptr) {
    unsigned int pos = 0;

    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            unsigned int j = 0;
            int k = 0;
            while (file_system[i].path[k] != '\0') {
                vidptr[pos] = file_system[i].path[k];
                vidptr[pos + 1] = 0x07;  // Light grey color
                ++k;
                pos += 2;
            }
            vidptr[pos] = '|';
            vidptr[pos + 1] = 0x07;  // Light grey color
            pos += 2;
            while (file_system[i].name[j] != '\0') {
                vidptr[pos] = file_system[i].name[j];
                vidptr[pos + 1] = 0x07;  // Light grey color
                ++j;
                pos += 2;
            }
            vidptr[pos] = ' ';
            vidptr[pos + 1] = 0x07; // Light grey color
            pos += 2;
        }
    }
}

// Access console state to save/restore cursor
extern ConsoleState console_state;

// Function to initialize the file system pop module
void filesystem_pop_func(unsigned int start_pos) {
    (void)start_pos;
    
    // Initialize the file system
    init_filesystem();
    
    // Save current console state
    unsigned int prev_x = console_state.cursor_x;
    unsigned int prev_y = console_state.cursor_y;
    unsigned char prev_color = console_state.current_color;
    
    // Write message at bottom left of screen
    console_set_cursor(0, 24);
    console_print_color("File Systems Ready", CONSOLE_SUCCESS_COLOR);
    
    // Restore previous cursor/color so normal typing is unaffected
    console_set_color(prev_color);
    console_set_cursor(prev_x, prev_y);
}

const PopModule filesystem_module = {
    .name = "filesystem",
    .message = "Filesystem Initialized, type 'help' for commands",
    .pop_function = filesystem_pop_func
};
