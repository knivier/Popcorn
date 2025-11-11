// src/includes/syscall.h
#ifndef SYSCALL_H
#define SYSCALL_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

// System call numbers
#define SYS_EXIT        0x01
#define SYS_READ        0x02
#define SYS_WRITE       0x03
#define SYS_OPEN        0x04
#define SYS_CLOSE       0x05
#define SYS_SEEK        0x06
#define SYS_GETPID      0x07
#define SYS_FORK        0x08
#define SYS_EXEC        0x09
#define SYS_WAIT        0x0A
#define SYS_MALLOC      0x0B
#define SYS_FREE        0x0C
#define SYS_MMAP        0x0D
#define SYS_MUNMAP      0x0E
#define SYS_GETTIME     0x0F
#define SYS_SLEEP       0x10
#define SYS_YIELD       0x11
#define SYS_GETCWD      0x12
#define SYS_CHDIR       0x13
#define SYS_STAT        0x14
#define SYS_IOCTL       0x15

// Maximum number of system calls
#define MAX_SYSCALLS    32

// System call result codes
#define SYSCALL_SUCCESS     0
#define SYSCALL_ERROR      -1
#define SYSCALL_EINVAL     -2
#define SYSCALL_ENOMEM     -3
#define SYSCALL_ENOENT     -4
#define SYSCALL_EACCES     -5
#define SYSCALL_EBUSY      -6
#define SYSCALL_EAGAIN     -7

// System call context structure
typedef struct {
    uint64_t rax;    // Return value
    uint64_t rdi;    // Argument 1
    uint64_t rsi;    // Argument 2
    uint64_t rdx;    // Argument 3
    uint64_t rcx;    // Argument 4
    uint64_t r8;     // Argument 5
    uint64_t r9;     // Argument 6
    uint64_t rsp;    // Stack pointer
    uint64_t rip;    // Instruction pointer
    uint64_t rflags; // CPU flags
    uint64_t cs;     // Code segment
    uint64_t ss;     // Stack segment
} syscall_context_t;

// System call handler function type
typedef int64_t (*syscall_handler_t)(syscall_context_t* ctx);

// System call table entry
typedef struct {
    uint32_t syscall_num;
    syscall_handler_t handler;
    const char* name;
    uint32_t flags;
} syscall_entry_t;

// System call flags
#define SYSCALL_FLAG_NONE       0x00
#define SYSCALL_FLAG_PRIVILEGED 0x01
#define SYSCALL_FLAG_BLOCKING   0x02
#define SYSCALL_FLAG_SIGNAL     0x04

// File mode constants (for stat)
#define S_IFMT   0170000  // File type mask
#define S_IFREG  0100000  // Regular file
#define S_IFDIR  0040000  // Directory
#define S_IFCHR  0020000  // Character device
#define S_IFBLK  0060000  // Block device
#define S_IFIFO  0010000  // FIFO
#define S_IFLNK  0120000  // Symbolic link
#define S_IFSOCK 0140000  // Socket

// Permission bits
#define S_IRUSR  00400  // Read permission, owner
#define S_IWUSR  00200  // Write permission, owner
#define S_IXUSR  00100  // Execute permission, owner
#define S_IRGRP  00040  // Read permission, group
#define S_IWGRP  00020  // Write permission, group
#define S_IXGRP  00010  // Execute permission, group
#define S_IROTH  00004  // Read permission, others
#define S_IWOTH  00002  // Write permission, others
#define S_IXOTH  00001  // Execute permission, others

// Stat structure (simplified version)
typedef struct {
    uint64_t st_dev;      // Device ID
    uint64_t st_ino;      // Inode number
    uint32_t st_mode;     // File type and mode
    uint32_t st_nlink;    // Number of hard links
    uint32_t st_uid;      // User ID of owner
    uint32_t st_gid;      // Group ID of owner
    uint64_t st_rdev;     // Device ID (if special file)
    uint64_t st_size;     // Total size in bytes
    uint64_t st_blksize;  // Block size for filesystem I/O
    uint64_t st_blocks;   // Number of 512-byte blocks allocated
    uint64_t st_atime;    // Time of last access
    uint64_t st_mtime;    // Time of last modification
    uint64_t st_ctime;    // Time of last status change
} stat_t;

// Function declarations
void syscall_init(void);
void syscall_register(uint32_t syscall_num, syscall_handler_t handler, const char* name, uint32_t flags);
int64_t syscall_dispatch(syscall_context_t* ctx);
void syscall_handler_asm(void); // Assembly entry point

// Individual system call handlers
int64_t sys_exit(syscall_context_t* ctx);
int64_t sys_read(syscall_context_t* ctx);
int64_t sys_write(syscall_context_t* ctx);
int64_t sys_open(syscall_context_t* ctx);
int64_t sys_close(syscall_context_t* ctx);
int64_t sys_seek(syscall_context_t* ctx);
int64_t sys_getpid(syscall_context_t* ctx);
int64_t sys_fork(syscall_context_t* ctx);
int64_t sys_exec(syscall_context_t* ctx);
int64_t sys_wait(syscall_context_t* ctx);
int64_t sys_malloc(syscall_context_t* ctx);
int64_t sys_free(syscall_context_t* ctx);
int64_t sys_mmap(syscall_context_t* ctx);
int64_t sys_munmap(syscall_context_t* ctx);
int64_t sys_gettime(syscall_context_t* ctx);
int64_t sys_sleep(syscall_context_t* ctx);
int64_t sys_yield(syscall_context_t* ctx);
int64_t sys_getcwd(syscall_context_t* ctx);
int64_t sys_chdir(syscall_context_t* ctx);
int64_t sys_stat(syscall_context_t* ctx);
int64_t sys_ioctl(syscall_context_t* ctx);

// Helper functions
bool syscall_is_valid(uint32_t syscall_num);
const char* syscall_get_name(uint32_t syscall_num);
void syscall_print_table(void);

#endif // SYSCALL_H
