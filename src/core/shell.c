#include "../includes/shell.h"
#include "../includes/console.h"
#include "../includes/pop_module.h"
#include "../includes/sysinfo_pop.h"
#include "../includes/memory_pop.h"
#include "../includes/cpu_pop.h"
#include "../includes/dolphin_pop.h"
#include "../includes/timer.h"
#include "../includes/scheduler.h"
#include "../includes/memory.h"
#include "../includes/syscall.h"
#include "../includes/utils.h"
#include "../includes/driver_abi.h"
#include "../includes/catalog.h"
#include "../includes/disk.h"
#include "../includes/kbd.h"
#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

#define HISTORY_SIZE 100
#define ENTER_KEY_CODE 0x1C

char command_history[HISTORY_SIZE][128];
unsigned int history_count = 0;

extern ConsoleState console_state;
extern unsigned int current_loc;
extern void write_port(unsigned short port, unsigned char data);

extern const PopModule filesystem_module;
extern const PopModule dolphin_module;

bool write_file(const char* name, const char* content);
const char* read_file(const char* name);
bool delete_file(const char* name);
void list_files(void);
void list_files_console(void);
bool create_directory(const char* name);
bool change_directory(const char* name);
void list_hierarchy(void);
const char* get_current_directory(void);
const char* search_file(const char* name);
bool copy_file(const char* src_name, const char* dest_path);
int get_tick_count(void);

void printTerm(const char *str, unsigned char color) {
    console_print_color(str, color);
}

/* Add command to history */
void add_to_history(const char *command) {
    if (command == NULL || command[0] == '\0') {
        return;
    }

    // Don't add duplicates of the last command
    if (history_count > 0) {
        bool is_duplicate = true;
        const char *last_cmd = command_history[history_count - 1];
        for (int i = 0; command[i] != '\0' || last_cmd[i] != '\0'; i++) {
            if (command[i] != last_cmd[i]) {
                is_duplicate = false;
                break;
            }
        }
        if (is_duplicate) {
            return;
        }
    }

    // Add to circular buffer
    unsigned int index = history_count % HISTORY_SIZE;
    strcpy_simple(command_history[index], command);

    if (history_count < HISTORY_SIZE) {
        history_count++;
    }
}

/* Get command from history */
const char* get_history_command(int offset) {
    if (history_count == 0 || offset < 0 || offset >= (int)history_count) {
        return NULL;
    }
    unsigned int index;
    if (history_count <= HISTORY_SIZE) {
        index = offset;
    } else {
        // Calculate actual position: start of oldest + offset
        unsigned int oldest = history_count % HISTORY_SIZE;
        index = (oldest + offset) % HISTORY_SIZE;
    }

    return command_history[index];
}

/* List of all commands for autocomplete */
static const char* available_commands[] = {
    "help", "halp", "clear", "uptime", "stop",
    "write", "read", "delete", "rm", "mkdir", "go", "back",
    "ls", "search", "cp", "listsys", "sysinfo",
    "mem", "mem -map", "mem -use", "mem -stats", "mem -info", "mem -debug",
    "cpu", "cpu -hz", "cpu -info",
    "tasks", "timer", "syscalls",
    "mon", "mon -debug", "mon -list", "mon -kill", "mon -ultramon",
    "dol", "dol -new", "dol -open", "dol -save", "dol -close", "dol -help",
    "drive", "drive list", "init_drive", "drive info", "drive cmd", "dev", "dev list",
    "catalog", "catalog list",
    "disk", "disk list", "disk use", "disk info", "disk read", "disk write",
    "install",
    "wrap", "wrap on", "wrap off",
    NULL
};

/* Parse number from string */
int parse_number(const char* str, uint32_t* result) {
    if (!str || !result) return 0;

    uint32_t num = 0;
    const char* start = str;

    while (*str >= '0' && *str <= '9') {
        num = num * 10 + (*str - '0');
        str++;
    }

    // Check if we parsed anything and if there are no trailing characters
    if (str == start || (*str != '\0' && *str != ' ')) {
        return 0;
    }

    *result = num;
    return 1;
}

/* Autocomplete command */
void autocomplete_command(char *buffer, unsigned int *index) {
    if (*index == 0) return;

    const char *match = NULL;
    int match_count = 0;

    // Find matching commands
    for (int i = 0; available_commands[i] != NULL; i++) {
        const char *cmd = available_commands[i];
        bool matches = true;

        // Check if command starts with buffer content
        for (unsigned int j = 0; j < *index; j++) {
            if (cmd[j] != buffer[j]) {
                matches = false;
                break;
            }
        }

        if (matches) {
            match = cmd;
            match_count++;
        }
    }

    // If exactly one match, autocomplete it
    if (match_count == 1) {
        size_t match_len = strlen_simple(match);
        for (size_t i = *index; i < match_len && i < 127; i++) {
            buffer[i] = match[i];
            console_putchar(match[i]);
        }
        *index = match_len;
        buffer[*index] = '\0';
    }
}

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
        console_println_color("Shell", CONSOLE_HEADER_COLOR);
        console_println("  clear   uptime   stop   wrap on|off");
        console_println_color("Files", CONSOLE_HEADER_COLOR);
        console_println("  ls  write  read  rm  mkdir  go  back  search  cp  listsys");
        console_println_color("System", CONSOLE_HEADER_COLOR);
        console_println("  sysinfo   mem   cpu   tasks   timer   syscalls");
        console_println("  mon -list|-debug|-kill|-ultramon");
        console_println_color("Devices", CONSOLE_HEADER_COLOR);
        console_println("  drive list   init_drive <name>   dev list   catalog");
        console_println("  disk list | use <name> | info | read <lba> | write <lba> <data>");
        console_println("  install <internal> | install <internal> YES");
        console_println_color("Editor / keys", CONSOLE_HEADER_COLOR);
        console_println("  dol -new|-open|-save|-help");
        console_println("  Up/Down scroll   Left/Right history");
    } else if (strcmp(command, "wrap") == 0 || strcmp(command, "wrap on") == 0) {
        console_set_wrap(true);
        console_print_success("wrap on");
    } else if (strcmp(command, "wrap off") == 0) {
        console_set_wrap(false);
        console_print_success("wrap off");
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
    } else if (strcmp(command, "stop") == 0) {
        console_print_warning("Shutting down...");
        write_port(0x64, 0xFE);  // Send reset command to keyboard controller
        // If shutdown fails, halt the CPU
        asm volatile("hlt");
    } else if (strncmp(command, "write ", 6) == 0) {
        // Validate that there is content after "write "
        if (command[6] == '\0' || command[6] == ' ') {
            console_print_error("Usage: write <filename> <content>");
            return;
        }
        
        char filename[21] = {0};
        char content[101] = {0};
        int i = 0;
        int j = 0;
        while (command[6 + i] != ' ' && i < 20 && command[6 + i] != '\0' && 6 + i < 128) {
            filename[i] = command[6 + i];
            i++;
        }
        filename[i] = '\0';
        
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        if (command[6 + i] == ' ' && 6 + i < 127) {
            i++;
            if (command[6 + i] == '\0' || 6 + i >= 128) {
                console_print_error("Content cannot be empty");
                return;
            }
            
            while (command[6 + i + j] != '\0' && j < 100 && 6 + i + j < 128) {
                content[j] = command[6 + i + j];
                j++;
            }
            content[j] = '\0';
            
            if (write_file(filename, content)) {
                console_print_success("File written successfully");
                console_print_color("Filename: ", CONSOLE_INFO_COLOR);
                console_println_color(filename, CONSOLE_FG_COLOR);
            } else {
                console_print_error("Failed to write file (content too long, name invalid, or filesystem full)");
            }
        } else {
            console_print_error("Invalid command format. Use: write <filename> <content>");
        }
    } else if (strncmp(command, "read ", 5) == 0) {
        if (command[5] == '\0' || command[5] == ' ') {
            console_print_error("Usage: read <filename>");
            return;
        }
        
        char filename[21] = {0};
        int i = 0;
        while (command[5 + i] != '\0' && command[5 + i] != ' ' && i < 20 && 5 + i < 128) {
            filename[i] = command[5 + i];
            i++;
        }
        filename[i] = '\0';
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        const char* content = read_file(filename);
        if (content) {
            console_print_color("File content: ", CONSOLE_INFO_COLOR);
            console_println_color(content, CONSOLE_FG_COLOR);
        } else {
            console_print_error("File not found or cannot be read");
        }
    } else if (strncmp(command, "delete ", 7) == 0) {
        if (command[7] == '\0' || command[7] == ' ') {
            console_print_error("Usage: delete <filename>");
            return;
        }
        
        char filename[21] = {0};
        int i = 0;
        while (command[7 + i] != '\0' && command[7 + i] != ' ' && i < 20 && 7 + i < 128) {
            filename[i] = command[7 + i];
            i++;
        }
        filename[i] = '\0';
        
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        if (delete_file(filename)) {
            console_print_success("File deleted successfully");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        } else {
            console_print_error("File not found or cannot be deleted");
        }
    } else if (strncmp(command, "mkdir ", 6) == 0) {
        // Validate that there is a directory name after "mkdir "
        if (command[6] == '\0' || command[6] == ' ') {
            console_print_error("Usage: mkdir <dirname>");
            return;
        }
        
        char dirname[21] = {0};
        int i = 0;
        while (command[6 + i] != '\0' && command[6 + i] != ' ' && i < 20 && 6 + i < 128) {
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
            console_print_error("Failed to create directory (already exists, name too long, or filesystem full)");
        }
    } else if (strncmp(command, "go ", 3) == 0) {
        // Validate that there is a directory name after "go "
        if (command[3] == '\0' || command[3] == ' ') {
            console_print_error("Usage: go <dirname>");
            return;
        }
        
        char dirname[21] = {0};
        int i = 0;
        while (command[3 + i] != '\0' && command[3 + i] != ' ' && i < 20 && 3 + i < 128) {
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
            console_print_error("Directory not found or cannot be accessed");
        }
    } else if (strncmp(command, "rm ", 3) == 0) {
        // rm command - alias for delete with better error handling
        if (command[3] == '\0' || command[3] == ' ') {
            console_print_error("Usage: rm <filename>");
            return;
        }
        
        char filename[21] = {0};
        int i = 0;
        while (command[3 + i] != '\0' && command[3 + i] != ' ' && i < 20 && 3 + i < 128) {
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
            console_print_error("File not found or cannot be removed");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        }
    } else if (strcmp(command, "back") == 0) {
        if (change_directory("back")) {
            console_print_success("Changed to parent directory");
        } else {
            console_print_error("Already at root directory");
        }
    } else if (strcmp(command, "ls") == 0) {
        list_files_console();
    } else if (strncmp(command, "search ", 7) == 0) {
        // Search command - find file and return its directory
        if (command[7] == '\0' || command[7] == ' ') {
            console_print_error("Usage: search <filename>");
            return;
        }
        
        char filename[21] = {0};
        int i = 0;
        while (command[7 + i] != '\0' && command[7 + i] != ' ' && i < 20 && 7 + i < 128) {
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
            console_print_error("File not found");
            console_print_color("Filename: ", CONSOLE_INFO_COLOR);
            console_println_color(filename, CONSOLE_FG_COLOR);
        }
    } else if (strncmp(command, "cp ", 3) == 0) {
        // Copy command - copy file to another directory
        if (command[3] == '\0' || command[3] == ' ') {
            console_print_error("Usage: cp <filename> <directory>");
            return;
        }
        
        char filename[21] = {0};
        char destdir[100] = {0}; // MAX_PATH_LENGTH from filesystem
        int i = 0;
        int j = 0;
        
        // Parse filename
        while (command[3 + i] != ' ' && i < 20 && command[3 + i] != '\0' && 3 + i < 128) {
            filename[i] = command[3 + i];
            i++;
        }
        filename[i] = '\0';
        
        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }
        
        // Skip spaces
        while (command[3 + i] == ' ' && 3 + i < 127) {
            i++;
        }
        
        // Parse destination directory
        if (command[3 + i] == '\0' || 3 + i >= 128) {
            console_print_error("Usage: cp <filename> <directory>");
            return;
        }
        
        while (command[3 + i + j] != '\0' && command[3 + i + j] != ' ' && j < 99 && 3 + i + j < 128) {
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
            console_print_error("Failed to copy file (not found, destination invalid, or already exists)");
        }
    } else if (strcmp(command, "listsys") == 0) {
        console_newline();
        console_println_color("File System Hierarchy:", CONSOLE_HEADER_COLOR);
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
        list_hierarchy();
        console_newline();
    } else if (strcmp(command, "sysinfo") == 0) {
        sysinfo_print_full();
    } else if (strncmp(command, "mem ", 4) == 0) {
        // Memory commands: mem -map, mem -use, mem -stats, mem -info, mem -debug
        if (strcmp(command + 4, "-map") == 0) {
            memory_print_map();
        } else if (strcmp(command + 4, "-use") == 0) {
            memory_print_usage();
        } else if (strcmp(command + 4, "-stats") == 0) {
            memory_print_stats();
        } else if (strcmp(command + 4, "-info") == 0) {
            kernel_memory_print_stats();
        } else if (strcmp(command + 4, "-debug") == 0) {
            memory_debug_print();
        } else {
            console_print_error("Unknown mem option. Use: -map, -use, -stats, -info, or -debug");
        }
    } else if (strcmp(command, "mem") == 0) {
        // Default: show usage
        memory_print_usage();
    } else if (strcmp(command, "tasks") == 0) {
        char buffer[64];
        console_newline();
        console_println_color("=== TASK INFORMATION ===", CONSOLE_HEADER_COLOR);
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
        
        TaskStruct* current = scheduler_get_current_task();
        if (current) {
            console_print_color("Current Task PID: ", CONSOLE_INFO_COLOR);
            int_to_str(current->pid, buffer);
            console_println_color(buffer, CONSOLE_FG_COLOR);
            
            console_print_color("Task State: ", CONSOLE_INFO_COLOR);
            switch (current->state) {
                case TASK_STATE_RUNNING:
                    console_println_color("Running", CONSOLE_SUCCESS_COLOR);
                    break;
                case TASK_STATE_READY:
                    console_println_color("Ready", CONSOLE_INFO_COLOR);
                    break;
                case TASK_STATE_BLOCKED:
                    console_println_color("Blocked", CONSOLE_WARNING_COLOR);
                    break;
                case TASK_STATE_SLEEPING:
                    console_println_color("Sleeping", CONSOLE_INFO_COLOR);
                    break;
                case TASK_STATE_ZOMBIE:
                    console_println_color("Zombie", CONSOLE_ERROR_COLOR);
                    break;
                default:
                    console_println_color("Unknown", CONSOLE_ERROR_COLOR);
                    break;
            }
            
            console_print_color("Priority: ", CONSOLE_INFO_COLOR);
            int_to_str(current->priority, buffer);
            console_println_color(buffer, CONSOLE_FG_COLOR);
            
            console_print_color("Total Runtime: ", CONSOLE_INFO_COLOR);
            int_to_str((int)current->total_runtime, buffer);
            console_println_color(buffer, CONSOLE_FG_COLOR);
        } else {
            console_println_color("No current task", CONSOLE_ERROR_COLOR);
        }
        
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    } else if (strcmp(command, "timer") == 0) {
        char buffer[64];
        console_newline();
        console_println_color("=== TIMER INFORMATION ===", CONSOLE_HEADER_COLOR);
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
        
        console_print_color("Timer Ticks: ", CONSOLE_INFO_COLOR);
        int_to_str((int)timer_get_ticks(), buffer);
        console_println_color(buffer, CONSOLE_FG_COLOR);
        
        console_print_color("Uptime (ms): ", CONSOLE_INFO_COLOR);
        int_to_str((int)timer_get_uptime_ms(), buffer);
        console_println_color(buffer, CONSOLE_FG_COLOR);
        
        console_print_color("Timer Frequency: ", CONSOLE_INFO_COLOR);
        int_to_str(TIMER_FREQUENCY, buffer);
        console_println_color(buffer, CONSOLE_FG_COLOR);
        
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);
    } else if (strcmp(command, "syscalls") == 0) {
        extern void syscall_print_table(void);
        syscall_print_table();
    } else if (strncmp(command, "mon ", 4) == 0) {
        // Task monitor commands: mon -debug, mon -list, mon -kill, mon -ultramon
        const char* args = command + 4;
        
        if (strcmp(args, "-debug") == 0) {
            // Start a debug task
            extern TaskStruct* scheduler_create_task(void (*function)(void), void* data, TaskPriority priority);
            extern void debug_task_function(void);
            extern uint32_t scheduler_get_task_count(void);
            
            uint32_t task_count = scheduler_get_task_count();
            if (task_count > 1) {
                console_print_error("Debug task already running. Use 'mon -kill [pid]' to stop it first.");
                return;
            }
            
            TaskStruct* task = scheduler_create_task(debug_task_function, NULL, PRIORITY_NORMAL);
            if (task) {
                console_print_success("Debug task started");
                console_print_color("PID: ", CONSOLE_INFO_COLOR);
                char buffer[32];
                int_to_str(task->pid, buffer);
                console_println_color(buffer, CONSOLE_FG_COLOR);
            } else {
                console_print_error("Failed to create debug task");
            }
        } else if (strncmp(args, "-debug ", 7) == 0) {
            // Start debug task with custom PID
            const char* pid_str = args + 7;
            uint32_t custom_pid;
            if (!parse_number(pid_str, &custom_pid)) {
                console_print_error("Invalid PID. Must be a positive number.");
                return;
            }
            
            extern TaskStruct* scheduler_create_task_with_pid(void (*function)(void), void* data, TaskPriority priority, uint32_t custom_pid);
            extern void debug_task_function(void);
            
            TaskStruct* task = scheduler_create_task_with_pid(debug_task_function, NULL, PRIORITY_NORMAL, custom_pid);
            if (task) {
                console_print_success("Debug task started with custom PID");
                console_print_color("PID: ", CONSOLE_INFO_COLOR);
                char buffer[32];
                int_to_str(task->pid, buffer);
                console_println_color(buffer, CONSOLE_FG_COLOR);
            } else {
                console_print_error("Failed to create debug task with custom PID");
            }
        } else if (strcmp(args, "-list") == 0) {
            // List all running tasks
            extern void scheduler_print_tasks(void);
            console_newline();
            console_println_color("=== TASK LIST ===", CONSOLE_HEADER_COLOR);
            scheduler_print_tasks();
        } else if (strncmp(args, "-kill ", 6) == 0) {
            // Kill specific task by PID
            const char* pid_str = args + 6;
            uint32_t pid;
            if (!parse_number(pid_str, &pid)) {
                console_print_error("Invalid PID. Must be a positive number.");
                return;
            }
            
            if (pid == 0) {
                console_print_error("Cannot kill idle task (PID 0)");
                return;
            }
            
            extern void scheduler_destroy_task(uint32_t pid);
            scheduler_destroy_task(pid);
            console_print_success("Task killed");
        } else if (strcmp(args, "-ultramon") == 0) {
            // Kill all tasks except idle
            extern void scheduler_kill_all_except_idle(void);
            extern void scheduler_print_tasks(void);
            
            console_print_warning("Killing all tasks except idle...");
            scheduler_kill_all_except_idle();
            console_print_success("All tasks killed except idle");
            
            console_newline();
            console_println_color("Remaining tasks:", CONSOLE_INFO_COLOR);
            scheduler_print_tasks();
        } else {
            console_print_error("Unknown mon option. Use: -debug, -debug [pid], -list, -kill [pid], or -ultramon");
        }
    } else if (strncmp(command, "cpu ", 4) == 0) {
        // CPU commands: cpu -hz, cpu -info
        if (strcmp(command + 4, "-hz") == 0) {
            cpu_print_frequency();
        } else if (strcmp(command + 4, "-info") == 0) {
            cpu_print_info();
        } else {
            console_print_error("Unknown cpu option. Use: -hz or -info");
        }
    } else if (strcmp(command, "cpu") == 0) {
        // Default: show info
        cpu_print_info();
    } else if (strncmp(command, "dol ", 4) == 0) {
        // Dolphin text editor commands
        if (strncmp(command + 4, "-new ", 5) == 0) {
            dolphin_new(command + 9);
        } else if (strncmp(command + 4, "-open ", 6) == 0) {
            dolphin_open(command + 10);
        } else if (strcmp(command + 4, "-save") == 0) {
            dolphin_save();
        } else if (strcmp(command + 4, "-close") == 0 || strcmp(command + 4, "-quit") == 0) {
            dolphin_close();
        } else if (strcmp(command + 4, "-quit!") == 0) {
            // Force quit without saving
            if (dolphin_is_active()) {
                EditorState* state = dolphin_get_state();
                state->active = false;
                console_clear();
                console_draw_header("Popcorn Kernel v0.5");
                console_println_color("Dolphin editor closed (unsaved changes discarded)", CONSOLE_WARNING_COLOR);
                console_newline();
                console_draw_prompt_with_path(get_current_directory());
            }
        } else if (strcmp(command + 4, "-help") == 0) {
            dolphin_help();
        } else {
            console_print_error("Unknown dol option. Use: -new, -open, -save, -close, -help");
        }
    } else if (strcmp(command, "dol") == 0) {
        // Default: show help
        dolphin_help();
    } else if (strcmp(command, "drive") == 0 || strcmp(command, "drive list") == 0
               || strcmp(command, "drv") == 0 || strcmp(command, "drv list") == 0) {
        char buf[256];
        list_drives(buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strncmp(command, "init_drive ", 11) == 0) {
        const char* name = command + 11;
        if (init_drive(name) == 0) {
            console_print_success("drive ready");
        } else {
            console_print_error("init_drive failed");
        }
    } else if (strncmp(command, "drv load ", 9) == 0) {
        /* Alias for older name. */
        if (init_drive(command + 9) == 0) {
            console_print_success("drive ready");
        } else {
            console_print_error("init_drive failed");
        }
    } else if (strncmp(command, "drive info ", 11) == 0) {
        char buf[128];
        drive_cmd(command + 11, "info", buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strncmp(command, "drv info ", 9) == 0) {
        char buf[128];
        drive_cmd(command + 9, "info", buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strncmp(command, "drive cmd ", 10) == 0 || strncmp(command, "drv cmd ", 8) == 0) {
        const char* rest = command[0] == 'd' && command[1] == 'r' && command[2] == 'i'
                               ? command + 10
                               : command + 8;
        char tmp[128];
        size_t n = 0;
        while (rest[n] && rest[n] != ' ' && n + 1 < sizeof(tmp)) {
            tmp[n] = rest[n];
            n++;
        }
        tmp[n] = '\0';
        const char* cmd = rest[n] == ' ' ? rest + n + 1 : "status";
        char buf[128];
        drive_cmd(tmp, cmd, buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strcmp(command, "dev") == 0 || strcmp(command, "dev list") == 0) {
        char buf[256];
        list_devices(buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strcmp(command, "catalog") == 0 || strcmp(command, "catalog list") == 0) {
        char buf[512];
        rust_catalog_list(CATALOG_KIND_ALL, buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strcmp(command, "disk") == 0 || strcmp(command, "disk list") == 0) {
        char buf[384];
        rust_disk_list(buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strncmp(command, "disk use ", 9) == 0) {
        if (rust_disk_use(command + 9) == 0) {
            console_print_success("disk selected");
        } else {
            console_print_error("disk use failed (unknown or LOCKED — install <name> YES)");
        }
    } else if (strncmp(command, "install ", 8) == 0) {
        const char* p = command + 8;
        char name[32];
        int ni = 0;
        while (*p && *p != ' ' && ni < (int)sizeof(name) - 1) {
            name[ni++] = *p++;
        }
        name[ni] = '\0';
        while (*p == ' ') {
            p++;
        }
        int yes = (strcmp(p, "YES") == 0);
        if (name[0] == '\0') {
            console_print_error("Usage: install <name> [YES]");
        } else {
            int rc = rust_disk_install(name, yes);
            if (rc == 0) {
                console_print_success("unlocked + selected (writes allowed)");
            } else if (rc == 1) {
                console_println_color("Confirm: install <name> YES  (ERASES TARGET)", CONSOLE_INFO_COLOR);
            } else {
                console_print_error("install failed (internal disks only; need driver)");
            }
        }
    } else if (strcmp(command, "disk info") == 0) {
        char buf[192];
        rust_disk_info(buf, sizeof(buf));
        console_println_color(buf, CONSOLE_INFO_COLOR);
    } else if (strncmp(command, "disk read ", 10) == 0) {
        uint32_t lba32 = 0;
        if (!parse_number(command + 10, &lba32)) {
            console_print_error("Usage: disk read <lba>");
        } else {
            uint8_t sec[512];
            int rc = rust_disk_read((uint64_t)lba32, sec, sizeof(sec));
            if (rc < 0) {
                console_print_error("disk read failed (select a disk first?)");
            } else {
                /* ASCII preview (printable) + continuous hex — not spaced byte pairs. */
                char ascii[17];
                char hex[33];
                int ai = 0;
                int hi = 0;
                const char* hx = "0123456789ABCDEF";
                for (int i = 0; i < 16; i++) {
                    uint8_t b = sec[i];
                    ascii[ai++] = (b >= 32 && b < 127) ? (char)b : '.';
                    hex[hi++] = hx[b >> 4];
                    hex[hi++] = hx[b & 0xF];
                }
                ascii[ai] = '\0';
                hex[hi] = '\0';
                console_print_color("text ", CONSOLE_INFO_COLOR);
                console_println_color(ascii, CONSOLE_FG_COLOR);
                console_print_color("hex  ", CONSOLE_INFO_COLOR);
                console_println_color(hex, CONSOLE_FG_COLOR);
            }
        }
    } else if (strncmp(command, "disk write ", 11) == 0) {
        /*
         * disk write <lba> [payload]
         *   payload hex:  DEADBEEF  or  DE AD BE EF
         *   payload text: POPCORN   (any non-hex → ASCII into sector)
         * Rest of the 512-byte sector is zero-filled.
         */
        const char* p = command + 11;
        uint32_t lba32 = 0;
        if (!parse_number(p, &lba32)) {
            console_print_error("Usage: disk write <lba> <hex|text>");
        } else {
            while (*p >= '0' && *p <= '9') {
                p++;
            }
            while (*p == ' ') {
                p++;
            }
            uint8_t sec[512];
            for (int i = 0; i < 512; i++) {
                sec[i] = 0;
            }
            int ok = 1;
            if (*p) {
                /* Hex if every non-space char is 0-9A-F; otherwise ASCII text. */
                int all_hex = 1;
                for (const char* q = p; *q; q++) {
                    char c = *q;
                    if (c == ' ') {
                        continue;
                    }
                    int hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') ||
                              (c >= 'A' && c <= 'F');
                    if (!hex) {
                        all_hex = 0;
                        break;
                    }
                }
                int bi = 0;
                if (all_hex) {
                    while (*p && bi < 512) {
                        while (*p == ' ') {
                            p++;
                        }
                        if (!*p) {
                            break;
                        }
                        char c1 = *p++;
                        while (*p == ' ') {
                            p++;
                        }
                        char c2 = *p ? *p++ : '0';
                        int h1 = (c1 >= '0' && c1 <= '9') ? c1 - '0'
                                 : (c1 >= 'a' && c1 <= 'f') ? c1 - 'a' + 10
                                 : (c1 >= 'A' && c1 <= 'F') ? c1 - 'A' + 10
                                 : -1;
                        int h2 = (c2 >= '0' && c2 <= '9') ? c2 - '0'
                                 : (c2 >= 'a' && c2 <= 'f') ? c2 - 'a' + 10
                                 : (c2 >= 'A' && c2 <= 'F') ? c2 - 'A' + 10
                                 : -1;
                        if (h1 < 0 || h2 < 0) {
                            console_print_error("bad hex (use DEADBEEF or text like POPCORN)");
                            ok = 0;
                            break;
                        }
                        sec[bi++] = (uint8_t)((h1 << 4) | h2);
                    }
                } else {
                    while (*p && bi < 512) {
                        sec[bi++] = (uint8_t)*p++;
                    }
                }
            }
            if (ok) {
                int rc = rust_disk_write((uint64_t)lba32, sec, sizeof(sec));
                if (rc == -5) {
                    console_print_error("disk LOCKED — install <name> YES first");
                } else if (rc < 0) {
                    console_print_error("disk write refused (disk use <name> first?)");
                } else {
                    console_print_success("sector written");
                }
            }
        }
    } else {
        console_print_error("Command not found");
        console_print_color("Command: ", CONSOLE_INFO_COLOR);
        console_println_color(command, CONSOLE_FG_COLOR);
        console_println_color("Type 'help' for available commands", CONSOLE_INFO_COLOR);
    }
}


