#include "includes/pop_module.h"
#include <stdbool.h>
#include <stddef.h> // Include for NULL

// Forward declaration of strrcmp
int strrcmp(const char* str1, const char* str2);

#define MAX_FILES 10
#define MAX_FILENAME_LENGTH 20
#define MAX_FILE_CONTENT_LENGTH 100
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
}

// Function to create a file
bool create_file(const char* name) {
    for (int i = 0; i < MAX_FILES; ++i) {
        if (!file_system[i].in_use) {
            int j = 0;
            while (name[j] != '\0' && j < MAX_FILENAME_LENGTH) {
                file_system[i].name[j] = name[j];
                j++;
            }
            file_system[i].name[j] = '\0';

            j = 0;
            while (current_path[j] != '\0' && j < MAX_PATH_LENGTH) {
                file_system[i].path[j] = current_path[j];
                j++;
            }
            file_system[i].path[j] = '\0';

            file_system[i].in_use = true;
            return true;
        }
    }
    return false;
}

// Function to write to a file
bool write_file(const char* name, const char* content) {
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            int j = 0;
            while (file_system[i].name[j] == name[j] && file_system[i].name[j] != '\0' && name[j] != '\0') {
                j++;
            }
            if (file_system[i].name[j] == '\0' && name[j] == '\0' && strrcmp(file_system[i].path, current_path) == 0) {
                j = 0;
                int k = 0;
                while (content[k] != '\0' && j < MAX_FILE_CONTENT_LENGTH) {
                    file_system[i].content[j] = content[k];
                    j++;
                    k++;
                }
                file_system[i].content[j] = '\0';
                return true;
            }
        }
    }
    return false;
}

// Function to read a file
const char* read_file(const char* name) {
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            int j = 0;
            while (file_system[i].name[j] == name[j] && file_system[i].name[j] != '\0' && name[j] != '\0') {
                j++;
            }
            if (file_system[i].name[j] == '\0' && name[j] == '\0' && strrcmp(file_system[i].path, current_path) == 0) {
                return file_system[i].content;
            }
        }
    }
    return NULL;
}

// Function to list all files in the current directory
void list_files() {
    char* vidptr = (char*)0xb8000;
    unsigned int pos = 0;

    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use && strrcmp(file_system[i].path, current_path) == 0) {
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

// Function to delete a file
bool delete_file(const char* name) {
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            int j = 0;
            while (file_system[i].name[j] == name[j] && file_system[i].name[j] != '\0' && name[j] != '\0') {
                j++;
            }
            if (file_system[i].name[j] == '\0' && name[j] == '\0' && strrcmp(file_system[i].path, current_path) == 0) {
                file_system[i].in_use = false;
                return true;
            }
        }
    }
    return false;
}

// Function to create a new directory
bool create_directory(const char* name) {
    char new_path[MAX_PATH_LENGTH];
    int i = 0;
    while (current_path[i] != '\0' && i < MAX_PATH_LENGTH) {
        new_path[i] = current_path[i];
        i++;
    }
    new_path[i] = '|';
    i++;
    int j = 0;
    while (name[j] != '\0' && i < MAX_PATH_LENGTH) {
        new_path[i] = name[j];
        i++;
        j++;
    }
    new_path[i] = '\0';

    for (int k = 0; k < MAX_FILES; ++k) {
        if (file_system[k].in_use && strrcmp(file_system[k].path, new_path) == 0 && strrcmp(file_system[k].name, name) == 0) {
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
    if (strrcmp(name, "back") == 0) {
        if (strrcmp(current_path, "root") == 0) {
            return true; // Already at root, cannot go back further
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
        return true;
    } else {
        char new_path[MAX_PATH_LENGTH];
        int i = 0;
        while (current_path[i] != '\0' && i < MAX_PATH_LENGTH) {
            new_path[i] = current_path[i];
            i++;
        }
        new_path[i] = '|';
        i++;
        int j = 0;
        while (name[j] != '\0' && i < MAX_PATH_LENGTH) {
            new_path[i] = name[j];
            i++;
            j++;
        }
        new_path[i] = '\0';

        for (int k = 0; k < MAX_FILES; ++k) {
            if (file_system[k].in_use && strrcmp(file_system[k].path, current_path) == 0 && strrcmp(file_system[k].name, name) == 0) {
                int l = 0;
                while (new_path[l] != '\0' && l < MAX_PATH_LENGTH) {
                    current_path[l] = new_path[l];
                    l++;
                }
                current_path[l] = '\0';
                return true;
            }
        }
        return false; // Directory does not exist
    }
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

// Function to initialize the file system pop module
void filesystem_pop_func(unsigned int start_pos) {
    char* vidptr = (char*)0xb8000;
    const char* msg = "Filesystem Initialized";
    unsigned int i = start_pos;
    unsigned int j = 0;

    // Display the initialization message on the screen
    while (msg[j] != '\0') {
        vidptr[i] = msg[j];
        vidptr[i + 1] = 0x02;  // Green color
        ++j;
        i = i + 2;
    }

    // Initialize the file system
    init_filesystem();

    // Example file system operations
    create_file("test.txt");
    write_file("test.txt", "Test.txt Activated!");
    const char* content = read_file("test.txt");
    if (content) {
        // Display the file content on the screen
        j = 0;
        while (content[j] != '\0') {
            vidptr[i] = content[j];
            vidptr[i + 1] = 0x02;  // Green color
            ++j;
            i = i + 2;
        }
    }
}

const PopModule filesystem_module = {
    .name = "filesystem",
    .message = "Filesystem Initialized, type 'help' for commands",
    .pop_function = filesystem_pop_func
};