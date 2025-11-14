// src/core/syscall.c
#include "../includes/syscall.h"
#include "../includes/console.h"
#include "../includes/memory.h"
#include "../includes/scheduler.h"
#include "../includes/timer.h"
#include "../includes/utils.h"
#include <stddef.h>

// Global system call table
static syscall_entry_t syscall_table[MAX_SYSCALLS];
static uint32_t syscall_count = 0;

// Current process context (simplified for now)
static uint32_t current_pid = 1;

// Initialize system call interface
void syscall_init(void) {
    // Clear system call table
    for (int i = 0; i < MAX_SYSCALLS; i++) {
        syscall_table[i].syscall_num = 0;
        syscall_table[i].handler = NULL;
        syscall_table[i].name = NULL;
        syscall_table[i].flags = 0;
    }
    syscall_count = 0;

    // Register basic system calls
    syscall_register(SYS_EXIT, sys_exit, "exit", SYSCALL_FLAG_NONE);
    syscall_register(SYS_READ, sys_read, "read", SYSCALL_FLAG_BLOCKING);
    syscall_register(SYS_WRITE, sys_write, "write", SYSCALL_FLAG_NONE);
    syscall_register(SYS_OPEN, sys_open, "open", SYSCALL_FLAG_NONE);
    syscall_register(SYS_CLOSE, sys_close, "close", SYSCALL_FLAG_NONE);
    syscall_register(SYS_SEEK, sys_seek, "seek", SYSCALL_FLAG_NONE);
    syscall_register(SYS_GETPID, sys_getpid, "getpid", SYSCALL_FLAG_NONE);
    syscall_register(SYS_FORK, sys_fork, "fork", SYSCALL_FLAG_PRIVILEGED);
    syscall_register(SYS_EXEC, sys_exec, "exec", SYSCALL_FLAG_PRIVILEGED);
    syscall_register(SYS_WAIT, sys_wait, "wait", SYSCALL_FLAG_BLOCKING);
    syscall_register(SYS_MALLOC, sys_malloc, "malloc", SYSCALL_FLAG_NONE);
    syscall_register(SYS_FREE, sys_free, "free", SYSCALL_FLAG_NONE);
    syscall_register(SYS_MMAP, sys_mmap, "mmap", SYSCALL_FLAG_PRIVILEGED);
    syscall_register(SYS_MUNMAP, sys_munmap, "munmap", SYSCALL_FLAG_PRIVILEGED);
    syscall_register(SYS_GETTIME, sys_gettime, "gettime", SYSCALL_FLAG_NONE);
    syscall_register(SYS_SLEEP, sys_sleep, "sleep", SYSCALL_FLAG_BLOCKING);
    syscall_register(SYS_YIELD, sys_yield, "yield", SYSCALL_FLAG_NONE);
    syscall_register(SYS_GETCWD, sys_getcwd, "getcwd", SYSCALL_FLAG_NONE);
    syscall_register(SYS_CHDIR, sys_chdir, "chdir", SYSCALL_FLAG_NONE);
    syscall_register(SYS_STAT, sys_stat, "stat", SYSCALL_FLAG_NONE);
    syscall_register(SYS_IOCTL, sys_ioctl, "ioctl", SYSCALL_FLAG_PRIVILEGED);

    console_println_color("System call interface initialized", CONSOLE_SUCCESS_COLOR);
}

// Register a system call
void syscall_register(uint32_t syscall_num, syscall_handler_t handler, const char* name, uint32_t flags) {
    if (syscall_count >= MAX_SYSCALLS) {
        console_println_color("ERROR: System call table full", CONSOLE_ERROR_COLOR);
        return;
    }

    syscall_table[syscall_count].syscall_num = syscall_num;
    syscall_table[syscall_count].handler = handler;
    syscall_table[syscall_count].name = name;
    syscall_table[syscall_count].flags = flags;
    syscall_count++;

    console_print_color("Registered syscall: ", CONSOLE_INFO_COLOR);
    console_print_color(name, CONSOLE_SUCCESS_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);
}

// Dispatch system call
int64_t syscall_dispatch(syscall_context_t* ctx) {
    if (!ctx) {
        return SYSCALL_EINVAL;
    }

    uint32_t syscall_num = (uint32_t)ctx->rax;
    
    // Find system call handler
    for (uint32_t i = 0; i < syscall_count; i++) {
        if (syscall_table[i].syscall_num == syscall_num) {
            if (syscall_table[i].handler) {
                return syscall_table[i].handler(ctx);
            }
        }
    }

    // Unknown system call
    console_print_color("ERROR: Unknown system call: ", CONSOLE_ERROR_COLOR);
    char buffer[16];
    int_to_str(syscall_num, buffer);
    console_println_color(buffer, CONSOLE_ERROR_COLOR);
    return SYSCALL_EINVAL;
}

// Check if system call number is valid
bool syscall_is_valid(uint32_t syscall_num) {
    for (uint32_t i = 0; i < syscall_count; i++) {
        if (syscall_table[i].syscall_num == syscall_num) {
            return true;
        }
    }
    return false;
}

// Get system call name
const char* syscall_get_name(uint32_t syscall_num) {
    for (uint32_t i = 0; i < syscall_count; i++) {
        if (syscall_table[i].syscall_num == syscall_num) {
            return syscall_table[i].name;
        }
    }
    return "unknown";
}

// Print system call table
void syscall_print_table(void) {
    console_println_color("=== SYSTEM CALL TABLE ===", CONSOLE_HEADER_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);
    
    for (uint32_t i = 0; i < syscall_count; i++) {
        char buffer[32];
        
        console_print_color("0x", CONSOLE_INFO_COLOR);
        int_to_str(syscall_table[i].syscall_num, buffer);
        console_print_color(buffer, CONSOLE_INFO_COLOR);
        console_print_color(": ", CONSOLE_FG_COLOR);
        console_print_color(syscall_table[i].name, CONSOLE_SUCCESS_COLOR);
        
        if (syscall_table[i].flags & SYSCALL_FLAG_PRIVILEGED) {
            console_print_color(" [PRIVILEGED]", CONSOLE_WARNING_COLOR);
        }
        if (syscall_table[i].flags & SYSCALL_FLAG_BLOCKING) {
            console_print_color(" [BLOCKING]", CONSOLE_INFO_COLOR);
        }
        
        console_println_color("", CONSOLE_FG_COLOR);
    }
}

// System call handlers

int64_t sys_exit(syscall_context_t* ctx) {
    int exit_code = (int)ctx->rdi;
    console_print_color("Process exit with code: ", CONSOLE_INFO_COLOR);
    char buffer[16];
    int_to_str(exit_code, buffer);
    console_println_color(buffer, CONSOLE_INFO_COLOR);
    return SYSCALL_SUCCESS;
}

int64_t sys_read(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    void* buf = (void*)ctx->rsi;
    size_t count = (size_t)ctx->rdx;
    
    // Suppress unused variable warnings for now
    (void)buf;
    (void)count;
    
    // Simplified: read from console for now
    if (fd == 0) { // stdin
        // This would normally read from user input
        // For now, return 0 (no data available)
        return 0;
    }
    
    return SYSCALL_EINVAL;
}

int64_t sys_write(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    const void* buf = (const void*)ctx->rsi;
    size_t count = (size_t)ctx->rdx;
    
    // Validate buffer pointer (basic check - in real kernel would check user space)
    if (!buf) {
        return SYSCALL_EINVAL;
    }
    
    // Limit maximum write size to prevent DoS
    if (count > 4096) {
        return SYSCALL_EINVAL;
    }
    
    // Simplified: write to console for now
    if (fd == 1 || fd == 2) { // stdout or stderr
        const char* str = (const char*)buf;
        for (size_t i = 0; i < count; i++) {
            console_putchar(str[i]);
        }
        return count;
    }
    
    return SYSCALL_EINVAL;
}

int64_t sys_open(syscall_context_t* ctx) {
    const char* pathname = (const char*)ctx->rdi;
    int flags = (int)ctx->rsi;
    
    // Suppress unused variable warning for now
    (void)flags;
    
    // Simplified file opening
    console_print_color("Opening file: ", CONSOLE_INFO_COLOR);
    console_println_color(pathname, CONSOLE_FG_COLOR);
    
    // Return a fake file descriptor
    return 3; // Start from 3 (0,1,2 are stdin,stdout,stderr)
}

int64_t sys_close(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    console_print_color("Closing file descriptor: ", CONSOLE_INFO_COLOR);
    char buffer[16];
    int_to_str(fd, buffer);
    console_println_color(buffer, CONSOLE_INFO_COLOR);
    return SYSCALL_SUCCESS;
}

int64_t sys_seek(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    int64_t offset = (int64_t)ctx->rsi;
    int whence = (int)ctx->rdx;
    
    // Suppress unused variable warning for now
    (void)whence;
    
    console_print_color("Seeking in fd ", CONSOLE_INFO_COLOR);
    char buffer[16];
    int_to_str(fd, buffer);
    console_print_color(buffer, CONSOLE_INFO_COLOR);
    console_println_color("", CONSOLE_FG_COLOR);
    
    return offset;
}

int64_t sys_getpid(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    return current_pid;
}

int64_t sys_fork(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    
    TaskStruct* current_task = scheduler_get_current_task();
    if (!current_task) {
        return SYSCALL_EINVAL;
    }
    
    // Create a new task that's a copy of the current one
    TaskStruct* child_task = scheduler_create_task(current_task->task_function, 
                                                   current_task->task_data, 
                                                   current_task->priority);
    
    if (!child_task) {
        return SYSCALL_ENOMEM;
    }
    
    // Set up parent-child relationship
    child_task->ppid = current_task->pid;
    
    // Copy the parent's context to child
    child_task->context = current_task->context;
    
    // Child gets PID 0 in return value, parent gets child's PID
    uint32_t child_pid = child_task->pid;
    
    // In the child process, return 0
    // In the parent process, return child's PID
    // For now, we'll return child's PID (parent's perspective)
    // The actual fork semantics would require modifying the return value
    // based on which process is executing
    
    console_print_color("Fork: Created child process PID ", CONSOLE_INFO_COLOR);
    char buffer[16];
    int_to_str(child_pid, buffer);
    console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
    
    return child_pid;
}

int64_t sys_exec(syscall_context_t* ctx) {
    const char* path = (const char*)ctx->rdi;
    console_print_color("Exec system call: ", CONSOLE_INFO_COLOR);
    console_println_color(path, CONSOLE_FG_COLOR);
    return SYSCALL_EINVAL;
}

int64_t sys_wait(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    
    TaskStruct* current_task = scheduler_get_current_task();
    if (!current_task) {
        return SYSCALL_EINVAL;
    }
    
    // Look for zombie children of current process
    for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
        TaskStruct* task = scheduler.ready_queue[priority];
        while (task) {
            if (task->ppid == current_task->pid && task->state == TASK_STATE_ZOMBIE) {
                uint32_t child_pid = task->pid;
                
                // Clean up the zombie child
                scheduler_destroy_task(child_pid);
                
                console_print_color("Wait: Reaped child process PID ", CONSOLE_INFO_COLOR);
                char buffer[16];
                int_to_str(child_pid, buffer);
                console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
                
                return child_pid;
            }
            task = task->next;
        }
    }
    
    // No zombie children found
    console_println_color("Wait: No zombie children found", CONSOLE_INFO_COLOR);
    return SYSCALL_EAGAIN; // No child processes to wait for
}

int64_t sys_malloc(syscall_context_t* ctx) {
    size_t size = (size_t)ctx->rdi;
    
    // Validate size: prevent zero-size allocation and check for overflow
    if (size == 0 || size > (1024 * 1024 * 1024)) {  // Max 1GB
        return SYSCALL_EINVAL;
    }
    
    void* ptr = kmalloc(size, MEM_ALLOC_NORMAL);
    
    if (ptr) {
        return (int64_t)ptr;
    }
    
    return SYSCALL_ENOMEM;
}

int64_t sys_free(syscall_context_t* ctx) {
    void* ptr = (void*)ctx->rdi;
    
    // kfree handles NULL pointers, so we allow it
    // In real kernel, would verify ptr was actually allocated by this process
    kfree(ptr);
    return SYSCALL_SUCCESS;
}

int64_t sys_mmap(syscall_context_t* ctx) {
    void* addr = (void*)ctx->rdi;
    size_t length = (size_t)ctx->rsi;
    int prot = (int)ctx->rdx;
    int flags = (int)ctx->rcx;
    int fd = (int)ctx->r8;
    int64_t offset = (int64_t)ctx->r9;
    
    // Suppress unused parameter warnings for now
    (void)addr;
    (void)prot;
    (void)flags;
    (void)fd;
    (void)offset;
    
    // For now, implement a simplified mmap that just allocates memory
    // In a real implementation, this would handle:
    // - File mapping (fd != -1)
    // - Anonymous mapping (fd == -1)
    // - Protection flags (PROT_READ, PROT_WRITE, PROT_EXEC)
    // - Mapping flags (MAP_PRIVATE, MAP_SHARED, MAP_ANONYMOUS)
    // - Address hints and alignment
    
    if (length == 0) {
        return SYSCALL_EINVAL;
    }
    
    // Validate length to prevent integer overflow and DoS
    if (length > (1024 * 1024 * 1024)) {  // Max 1GB
        return SYSCALL_EINVAL;
    }
    
    // Allocate memory using our memory manager
    void* mapped_addr = kmalloc(length, MEM_ALLOC_NORMAL);
    
    if (!mapped_addr) {
        return SYSCALL_ENOMEM;
    }
    
    // Zero the allocated memory
    memory_zero(mapped_addr, length);
    
    console_print_color("Mmap: Mapped ", CONSOLE_INFO_COLOR);
    char buffer[16];
    int_to_str((int)length, buffer);
    console_print_color(buffer, CONSOLE_INFO_COLOR);
    console_println_color(" bytes", CONSOLE_SUCCESS_COLOR);
    
    return (int64_t)mapped_addr;
}

int64_t sys_munmap(syscall_context_t* ctx) {
    void* addr = (void*)ctx->rdi;
    size_t length = (size_t)ctx->rsi;
    
    if (!addr || length == 0) {
        return SYSCALL_EINVAL;
    }
    
    // For now, implement a simplified munmap that just frees memory
    // In a real implementation, this would handle:
    // - Partial unmapping (length < mapped length)
    // - Page alignment requirements
    // - File-backed vs anonymous mappings
    // - Shared memory considerations
    
    // Free the memory using our memory manager
    kfree(addr);
    
    console_print_color("Munmap: Unmapped ", CONSOLE_INFO_COLOR);
    char buffer[16];
    int_to_str((int)length, buffer);
    console_print_color(buffer, CONSOLE_INFO_COLOR);
    console_println_color(" bytes", CONSOLE_SUCCESS_COLOR);
    
    return SYSCALL_SUCCESS;
}

int64_t sys_gettime(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    return timer_get_uptime_ms();
}

int64_t sys_sleep(syscall_context_t* ctx) {
    uint32_t ms = (uint32_t)ctx->rdi;
    console_print_color("Sleep for ", CONSOLE_INFO_COLOR);
    char buffer[16];
    int_to_str(ms, buffer);
    console_print_color(buffer, CONSOLE_INFO_COLOR);
    console_println_color(" ms", CONSOLE_INFO_COLOR);
    return SYSCALL_SUCCESS;
}

int64_t sys_yield(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    scheduler_yield();
    return SYSCALL_SUCCESS;
}

int64_t sys_getcwd(syscall_context_t* ctx) {
    char* buf = (char*)ctx->rdi;
    size_t size = (size_t)ctx->rsi;
    
    // Validate buffer pointer
    if (!buf) {
        return SYSCALL_EINVAL;
    }
    
    // Simplified: return current directory
    const char* cwd = "/";
    size_t len = 1;  // Length of "/" without null terminator
    
    // Need space for string plus null terminator
    if (len + 1 > size) {
        return SYSCALL_EINVAL;
    }
    
    for (size_t i = 0; i < len; i++) {
        buf[i] = cwd[i];
    }
    buf[len] = '\0';
    
    return len;
}

int64_t sys_chdir(syscall_context_t* ctx) {
    const char* path = (const char*)ctx->rdi;
    console_print_color("Changing directory to: ", CONSOLE_INFO_COLOR);
    console_println_color(path, CONSOLE_FG_COLOR);
    return SYSCALL_SUCCESS;
}

int64_t sys_stat(syscall_context_t* ctx) {
    const char* pathname = (const char*)ctx->rdi;
    stat_t* statbuf = (stat_t*)ctx->rsi;
    
    if (!pathname || !statbuf) {
        return SYSCALL_EINVAL;
    }
    
    // For now, implement a simplified stat that works with our filesystem
    // In a real implementation, this would:
    // - Parse the filesystem to find the file
    // - Fill in all stat fields properly
    // - Handle different file types
    // - Check permissions
    
    // Try to find the file in our filesystem
    extern const char* read_file(const char* name);
    const char* content = read_file(pathname);
    
    if (!content) {
        return SYSCALL_ENOENT; // File not found
    }
    
    // Fill in basic stat information
    statbuf->st_dev = 1;        // Device ID (simplified)
    statbuf->st_ino = (uint64_t)pathname; // Use pathname as inode (simplified)
    statbuf->st_mode = S_IFREG | S_IRUSR | S_IWUSR | S_IRGRP | S_IROTH; // Regular file with read/write for owner
    statbuf->st_nlink = 1;      // Single hard link
    statbuf->st_uid = 0;        // Root user
    statbuf->st_gid = 0;        // Root group
    statbuf->st_rdev = 0;       // Not a device file
    statbuf->st_blksize = 512;  // Standard block size
    statbuf->st_blocks = 0;     // Will calculate below
    
    // Calculate file size
    size_t file_size = 0;
    while (content[file_size] != '\0') {
        file_size++;
    }
    statbuf->st_size = file_size;
    statbuf->st_blocks = (file_size + 511) / 512; // Convert to 512-byte blocks
    
    // Set timestamps (simplified - use current time)
    uint64_t current_time = timer_get_uptime_ms();
    statbuf->st_atime = current_time;
    statbuf->st_mtime = current_time;
    statbuf->st_ctime = current_time;
    
    console_print_color("Stat: Retrieved info for ", CONSOLE_INFO_COLOR);
    console_println_color(pathname, CONSOLE_SUCCESS_COLOR);
    
    return SYSCALL_SUCCESS;
}

int64_t sys_ioctl(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    uint64_t request = ctx->rsi;
    void* argp = (void*)ctx->rdx;
    
    // For now, implement a simplified ioctl that handles basic device operations
    // In a real implementation, this would:
    // - Route to appropriate device driver based on fd
    // - Handle device-specific commands
    // - Validate permissions
    // - Handle different device types (TTY, network, etc.)
    
    // Handle console/TTY operations (fd 0, 1, 2)
    if (fd >= 0 && fd <= 2) {
        switch (request) {
            case 0x5401: // TCGETS - Get terminal attributes
                console_println_color("Ioctl: TCGETS (get terminal attributes)", CONSOLE_INFO_COLOR);
                return SYSCALL_SUCCESS;
                
            case 0x5402: // TCSETS - Set terminal attributes
                console_println_color("Ioctl: TCSETS (set terminal attributes)", CONSOLE_INFO_COLOR);
                return SYSCALL_SUCCESS;
                
            case 0x540B: // TIOCGWINSZ - Get window size
                if (argp) {
                    // Validate pointer before writing (basic check)
                    // In real kernel, would verify argp is in user space
                    uint16_t* winsize = (uint16_t*)argp;
                    winsize[0] = 80;  // rows
                    winsize[1] = 25;  // cols
                    winsize[2] = 0;   // x pixels (not used)
                    winsize[3] = 0;   // y pixels (not used)
                }
                console_println_color("Ioctl: TIOCGWINSZ (get window size)", CONSOLE_INFO_COLOR);
                return SYSCALL_SUCCESS;
                
            default:
                console_print_color("Ioctl: Unknown request ", CONSOLE_WARNING_COLOR);
                char buffer[16];
                int_to_str((int)request, buffer);
                console_println_color(buffer, CONSOLE_WARNING_COLOR);
                return SYSCALL_EINVAL;
        }
    }
    
    // Handle file descriptor operations
    if (fd >= 3) {
        console_print_color("Ioctl: File descriptor ", CONSOLE_INFO_COLOR);
        char buffer[16];
        int_to_str(fd, buffer);
        console_print_color(buffer, CONSOLE_INFO_COLOR);
        console_print_color(" request ", CONSOLE_INFO_COLOR);
        int_to_str((int)request, buffer);
        console_println_color(buffer, CONSOLE_SUCCESS_COLOR);
        return SYSCALL_SUCCESS;
    }
    
    return SYSCALL_EINVAL;
}
