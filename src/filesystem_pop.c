#include "includes/pop_module.h"
#include <stdbool.h>
#include <stddef.h> // Include for NULL

#define MAX_FILES 10
#define MAX_FILENAME_LENGTH 20
#define MAX_FILE_CONTENT_LENGTH 100

// Structure to represent a file
typedef struct {
    char name[MAX_FILENAME_LENGTH];
    char content[MAX_FILE_CONTENT_LENGTH];
    bool in_use;
} File;

// Simple in-memory file system
File file_system[MAX_FILES];

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
            if (file_system[i].name[j] == '\0' && name[j] == '\0') {
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
            if (file_system[i].name[j] == '\0' && name[j] == '\0') {
                return file_system[i].content;
            }
        }
    }
    return NULL;
}

// Function to delete a file
bool delete_file(const char* name) {
    for (int i = 0; i < MAX_FILES; ++i) {
        if (file_system[i].in_use) {
            int j = 0;
            while (file_system[i].name[j] == name[j] && file_system[i].name[j] != '\0' && name[j] != '\0') {
                j++;
            }
            if (file_system[i].name[j] == '\0' && name[j] == '\0') {
                file_system[i].in_use = false;
                return true;
            }
        }
    }
    return false;
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