// src/core/syscall.c — real handlers only (toys removed).
#include "../includes/syscall.h"
#include "../includes/console.h"
#include "../includes/memory.h"
#include "../includes/scheduler.h"
#include "../includes/timer.h"
#include "../includes/utils.h"
#include "../includes/device.h"
#include <stddef.h>

extern const char* get_current_directory(void);
extern bool change_directory(const char* name);

static syscall_entry_t syscall_table[MAX_SYSCALLS];
static uint32_t syscall_count = 0;
static uint32_t current_pid = 1;

static FileDesc* current_fds(void) {
    TaskStruct* t = scheduler_get_current_task();
    if (!t) {
        return NULL;
    }
    return t->fds;
}

void syscall_init(void) {
    for (int i = 0; i < MAX_SYSCALLS; i++) {
        syscall_table[i].syscall_num = 0;
        syscall_table[i].handler = NULL;
        syscall_table[i].name = NULL;
        syscall_table[i].flags = 0;
    }
    syscall_count = 0;

    /* Device / fd path (Phase 2). */
    syscall_register(SYS_READ, sys_read, "read", SYSCALL_FLAG_BLOCKING);
    syscall_register(SYS_WRITE, sys_write, "write", SYSCALL_FLAG_NONE);
    syscall_register(SYS_OPEN, sys_open, "open", SYSCALL_FLAG_NONE);
    syscall_register(SYS_CLOSE, sys_close, "close", SYSCALL_FLAG_NONE);
    syscall_register(SYS_IOCTL, sys_ioctl, "ioctl", SYSCALL_FLAG_NONE);

    /* Process / time (real). */
    syscall_register(SYS_EXIT, sys_exit, "exit", SYSCALL_FLAG_NONE);
    syscall_register(SYS_GETPID, sys_getpid, "getpid", SYSCALL_FLAG_NONE);
    syscall_register(SYS_GETTIME, sys_gettime, "gettime", SYSCALL_FLAG_NONE);
    syscall_register(SYS_SLEEP, sys_sleep, "sleep", SYSCALL_FLAG_BLOCKING);
    syscall_register(SYS_YIELD, sys_yield, "yield", SYSCALL_FLAG_NONE);

    /* Heap (kmalloc bridge). */
    syscall_register(SYS_MALLOC, sys_malloc, "malloc", SYSCALL_FLAG_NONE);
    syscall_register(SYS_FREE, sys_free, "free", SYSCALL_FLAG_NONE);

    /* Working-directory helpers (backed by the Rust FAT32 driver). */
    syscall_register(SYS_GETCWD, sys_getcwd, "getcwd", SYSCALL_FLAG_NONE);
    syscall_register(SYS_CHDIR, sys_chdir, "chdir", SYSCALL_FLAG_NONE);

    console_println_color("System call interface initialized", CONSOLE_SUCCESS_COLOR);
}

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
}

int64_t syscall_dispatch(syscall_context_t* ctx) {
    if (!ctx) {
        return SYSCALL_EINVAL;
    }
    uint32_t syscall_num = (uint32_t)ctx->rax;
    for (uint32_t i = 0; i < syscall_count; i++) {
        if (syscall_table[i].syscall_num == syscall_num && syscall_table[i].handler) {
            return syscall_table[i].handler(ctx);
        }
    }
    return SYSCALL_EINVAL;
}

bool syscall_is_valid(uint32_t syscall_num) {
    for (uint32_t i = 0; i < syscall_count; i++) {
        if (syscall_table[i].syscall_num == syscall_num) {
            return true;
        }
    }
    return false;
}

const char* syscall_get_name(uint32_t syscall_num) {
    for (uint32_t i = 0; i < syscall_count; i++) {
        if (syscall_table[i].syscall_num == syscall_num) {
            return syscall_table[i].name ? syscall_table[i].name : "?";
        }
    }
    return NULL;
}

void syscall_print_table(void) {
    char buffer[16];
    console_newline();
    console_println_color("=== SYSTEM CALL TABLE ===", CONSOLE_HEADER_COLOR);
    for (uint32_t i = 0; i < syscall_count; i++) {
        console_print_color("0x", CONSOLE_INFO_COLOR);
        int_to_str((int)syscall_table[i].syscall_num, buffer);
        console_print_color(buffer, CONSOLE_INFO_COLOR);
        console_print_color(": ", CONSOLE_FG_COLOR);
        console_println_color(syscall_table[i].name ? syscall_table[i].name : "?", CONSOLE_SUCCESS_COLOR);
    }
}

int64_t sys_exit(syscall_context_t* ctx) {
    (void)ctx;
    return SYSCALL_SUCCESS;
}

int64_t sys_read(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    void* buf = (void*)ctx->rsi;
    size_t count = (size_t)ctx->rdx;
    FileDesc* fds = current_fds();
    Device* dev = fd_get(fds, fd);
    if (!dev || !dev->ops || !dev->ops->read || !buf) {
        return SYSCALL_EINVAL;
    }
    return dev->ops->read(dev, buf, count);
}

int64_t sys_write(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    const void* buf = (const void*)ctx->rsi;
    size_t count = (size_t)ctx->rdx;
    FileDesc* fds = current_fds();
    Device* dev = fd_get(fds, fd);
    if (!dev || !dev->ops || !dev->ops->write || !buf) {
        return SYSCALL_EINVAL;
    }
    if (count > 4096) {
        return SYSCALL_EINVAL;
    }
    return dev->ops->write(dev, buf, count);
}

int64_t sys_open(syscall_context_t* ctx) {
    const char* pathname = (const char*)ctx->rdi;
    int flags = (int)ctx->rsi;
    FileDesc* fds = current_fds();
    if (!pathname || !fds) {
        return SYSCALL_EINVAL;
    }
    Device* dev = device_find(pathname);
    if (!dev) {
        return SYSCALL_ENOENT;
    }
    int fd = fd_alloc(fds, dev, (uint32_t)flags);
    if (fd < 0) {
        return SYSCALL_ENOMEM;
    }
    return fd;
}

int64_t sys_close(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    FileDesc* fds = current_fds();
    if (!fds || fd_close(fds, fd) != 0) {
        return SYSCALL_EINVAL;
    }
    return SYSCALL_SUCCESS;
}

int64_t sys_getpid(syscall_context_t* ctx) {
    (void)ctx;
    TaskStruct* t = scheduler_get_current_task();
    if (t) {
        return (int64_t)t->pid;
    }
    return (int64_t)current_pid;
}

int64_t sys_malloc(syscall_context_t* ctx) {
    size_t size = (size_t)ctx->rdi;
    if (size == 0 || size > (1024 * 1024 * 1024)) {
        return SYSCALL_EINVAL;
    }
    void* ptr = kmalloc(size, MEM_ALLOC_NORMAL);
    return ptr ? (int64_t)ptr : SYSCALL_ENOMEM;
}

int64_t sys_free(syscall_context_t* ctx) {
    void* ptr = (void*)ctx->rdi;
    if (!ptr) {
        return SYSCALL_SUCCESS;
    }
    if (!is_valid_allocation(ptr)) {
        return SYSCALL_EINVAL;
    }
    kfree(ptr);
    return SYSCALL_SUCCESS;
}

int64_t sys_gettime(syscall_context_t* ctx) {
    (void)ctx;
    return (int64_t)timer_get_uptime_ms();
}

int64_t sys_sleep(syscall_context_t* ctx) {
    scheduler_sleep_ms((uint32_t)ctx->rdi);
    return SYSCALL_SUCCESS;
}

int64_t sys_yield(syscall_context_t* ctx) {
    (void)ctx;
    scheduler_yield();
    return SYSCALL_SUCCESS;
}

int64_t sys_getcwd(syscall_context_t* ctx) {
    char* buf = (char*)ctx->rdi;
    size_t size = (size_t)ctx->rsi;
    if (!buf || size == 0) {
        return SYSCALL_EINVAL;
    }
    const char* cwd = get_current_directory();
    if (!cwd) {
        cwd = "/";
    }
    size_t len = 0;
    while (cwd[len]) {
        len++;
    }
    if (len + 1 > size) {
        return SYSCALL_EINVAL;
    }
    for (size_t i = 0; i < len; i++) {
        buf[i] = cwd[i];
    }
    buf[len] = '\0';
    return (int64_t)len;
}

int64_t sys_chdir(syscall_context_t* ctx) {
    const char* path = (const char*)ctx->rdi;
    if (!path) {
        return SYSCALL_EINVAL;
    }
    return change_directory(path) ? SYSCALL_SUCCESS : SYSCALL_ENOENT;
}

int64_t sys_ioctl(syscall_context_t* ctx) {
    int fd = (int)ctx->rdi;
    uint64_t request = ctx->rsi;
    void* argp = (void*)ctx->rdx;
    FileDesc* fds = current_fds();
    Device* dev = fd_get(fds, fd);
    if (!dev || !dev->ops || !dev->ops->ioctl) {
        return SYSCALL_EINVAL;
    }
    int64_t rc = dev->ops->ioctl(dev, request, argp);
    return (rc < 0) ? SYSCALL_EINVAL : rc;
}
