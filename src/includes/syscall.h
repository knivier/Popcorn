// src/includes/syscall.h — registered syscalls only (toys removed).
#ifndef SYSCALL_H
#define SYSCALL_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

/* Registered numbers (see syscall_init). Reserved holes kept for ABI stability. */
#define SYS_EXIT        0x01
#define SYS_READ        0x02
#define SYS_WRITE       0x03
#define SYS_OPEN        0x04
#define SYS_CLOSE       0x05
/* 0x06 seek — reserved, not registered */
#define SYS_GETPID      0x07
/* 0x08–0x0A fork/exec/wait — reserved */
#define SYS_MALLOC      0x0B
#define SYS_FREE        0x0C
/* 0x0D–0x0E mmap/munmap — reserved */
#define SYS_GETTIME     0x0F
#define SYS_SLEEP       0x10
#define SYS_YIELD       0x11
#define SYS_GETCWD      0x12
#define SYS_CHDIR       0x13
/* 0x14 stat — reserved */
#define SYS_IOCTL       0x15

#define MAX_SYSCALLS    32

#define SYSCALL_SUCCESS     0
#define SYSCALL_ERROR      -1
#define SYSCALL_EINVAL     -2
#define SYSCALL_ENOMEM     -3
#define SYSCALL_ENOENT     -4
#define SYSCALL_EACCES     -5
#define SYSCALL_EBUSY      -6
#define SYSCALL_EAGAIN     -7

typedef struct {
    uint64_t rax;
    uint64_t rdi;
    uint64_t rsi;
    uint64_t rdx;
    uint64_t rcx;
    uint64_t r8;
    uint64_t r9;
    uint64_t rsp;
    uint64_t rip;
    uint64_t rflags;
    uint64_t cs;
    uint64_t ss;
} syscall_context_t;

typedef int64_t (*syscall_handler_t)(syscall_context_t* ctx);

typedef struct {
    uint32_t syscall_num;
    syscall_handler_t handler;
    const char* name;
    uint32_t flags;
} syscall_entry_t;

#define SYSCALL_FLAG_NONE       0x00
#define SYSCALL_FLAG_PRIVILEGED 0x01
#define SYSCALL_FLAG_BLOCKING   0x02
#define SYSCALL_FLAG_SIGNAL     0x04

void syscall_init(void);
void syscall_register(uint32_t syscall_num, syscall_handler_t handler, const char* name, uint32_t flags);
int64_t syscall_dispatch(syscall_context_t* ctx);
void syscall_handler_asm(void);

int64_t sys_exit(syscall_context_t* ctx);
int64_t sys_read(syscall_context_t* ctx);
int64_t sys_write(syscall_context_t* ctx);
int64_t sys_open(syscall_context_t* ctx);
int64_t sys_close(syscall_context_t* ctx);
int64_t sys_getpid(syscall_context_t* ctx);
int64_t sys_malloc(syscall_context_t* ctx);
int64_t sys_free(syscall_context_t* ctx);
int64_t sys_gettime(syscall_context_t* ctx);
int64_t sys_sleep(syscall_context_t* ctx);
int64_t sys_yield(syscall_context_t* ctx);
int64_t sys_getcwd(syscall_context_t* ctx);
int64_t sys_chdir(syscall_context_t* ctx);
int64_t sys_ioctl(syscall_context_t* ctx);

bool syscall_is_valid(uint32_t syscall_num);
const char* syscall_get_name(uint32_t syscall_num);
void syscall_print_table(void);

#endif // SYSCALL_H
