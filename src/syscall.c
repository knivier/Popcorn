// src/syscall.c
#include "includes/syscall.h"
#include "includes/console.h"
#include "includes/memory.h"
#include "includes/scheduler.h"
#include "includes/timer.h"
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
    extern void int_to_str(int num, char *str);
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
        extern void int_to_str(int num, char *str);
        
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
    extern void int_to_str(int num, char *str);
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
    extern void int_to_str(int num, char *str);
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
    extern void int_to_str(int num, char *str);
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
    console_println_color("Fork system call (not implemented)", CONSOLE_WARNING_COLOR);
    return SYSCALL_EINVAL;
}

int64_t sys_exec(syscall_context_t* ctx) {
    const char* path = (const char*)ctx->rdi;
    console_print_color("Exec system call: ", CONSOLE_INFO_COLOR);
    console_println_color(path, CONSOLE_FG_COLOR);
    return SYSCALL_EINVAL;
}

int64_t sys_wait(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    console_println_color("Wait system call (not implemented)", CONSOLE_WARNING_COLOR);
    return SYSCALL_EINVAL;
}

int64_t sys_malloc(syscall_context_t* ctx) {
    size_t size = (size_t)ctx->rdi;
    void* ptr = kmalloc(size, MEM_ALLOC_NORMAL);
    
    if (ptr) {
        return (int64_t)ptr;
    }
    
    return SYSCALL_ENOMEM;
}

int64_t sys_free(syscall_context_t* ctx) {
    void* ptr = (void*)ctx->rdi;
    kfree(ptr);
    return SYSCALL_SUCCESS;
}

int64_t sys_mmap(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    console_println_color("Mmap system call (not implemented)", CONSOLE_WARNING_COLOR);
    return SYSCALL_EINVAL;
}

int64_t sys_munmap(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    console_println_color("Munmap system call (not implemented)", CONSOLE_WARNING_COLOR);
    return SYSCALL_EINVAL;
}

int64_t sys_gettime(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    return timer_get_uptime_ms();
}

int64_t sys_sleep(syscall_context_t* ctx) {
    uint32_t ms = (uint32_t)ctx->rdi;
    console_print_color("Sleep for ", CONSOLE_INFO_COLOR);
    char buffer[16];
    extern void int_to_str(int num, char *str);
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
    
    // Simplified: return current directory
    const char* cwd = "/";
    size_t len = 1;
    
    if (len >= size) {
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
    (void)ctx; // Suppress unused parameter warning
    console_println_color("Stat system call (not implemented)", CONSOLE_WARNING_COLOR);
    return SYSCALL_EINVAL;
}

int64_t sys_ioctl(syscall_context_t* ctx) {
    (void)ctx; // Suppress unused parameter warning
    console_println_color("Ioctl system call (not implemented)", CONSOLE_WARNING_COLOR);
    return SYSCALL_EINVAL;
}
