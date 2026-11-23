// src/includes/scheduler.h
#ifndef SCHEDULER_H
#define SCHEDULER_H

#include <stdint.h>
#include <stdbool.h>
#include "vmm.h"
#include "device.h"

// Task states
typedef enum {
    TASK_STATE_RUNNING,
    TASK_STATE_READY,
    TASK_STATE_BLOCKED,
    TASK_STATE_SLEEPING,
    TASK_STATE_ZOMBIE
} TaskState;

// Task priority levels
typedef enum {
    PRIORITY_IDLE = 0,
    PRIORITY_LOW = 1,
    PRIORITY_NORMAL = 2,
    PRIORITY_HIGH = 3,
    PRIORITY_REALTIME = 4
} TaskPriority;

/*
 * Per-task CPU context. Only `rsp` is live: it holds the task's saved kernel stack
 * pointer while it is switched out (switch_to() keeps rbx/rbp/r12-r15 and the return
 * address on that stack). The other fields record the initial entry state for
 * diagnostics and are not loaded by the switch.
 */
typedef struct {
    // General purpose registers (x86-64 calling convention)
    uint64_t r15;
    uint64_t r14;
    uint64_t r13;
    uint64_t r12;
    uint64_t r11;
    uint64_t r10;
    uint64_t r9;
    uint64_t r8;
    uint64_t rdi;  // 1st argument
    uint64_t rsi;  // 2nd argument
    uint64_t rbp;  // base pointer
    uint64_t rdx;  // 3rd argument
    uint64_t rcx;  // 4th argument
    uint64_t rbx;  // callee-saved
    uint64_t rax;  // return value
    
    // Control registers
    uint64_t rip;  // instruction pointer
    uint64_t rsp;  // stack pointer
    uint64_t rflags; // flags register
    
    // Segment registers (simplified for long mode)
    uint64_t cs;   // code segment
    uint64_t ss;   // stack segment
    uint64_t ds;   // data segment
    uint64_t es;   // extra segment
    uint64_t fs;   // extra segment
    uint64_t gs;   // extra segment
    
    /* Reserved, never loaded or saved: switch_to() switches at a call boundary where
     * SysV treats XMM/x87 as caller-saved, and IRQ stubs (idt.asm) fxsave/fxrstor the
     * interrupted state on the interrupted task's own stack. Kept for layout stability. */
    uint64_t fpu_state[32];
    uint64_t fpu_control;
} CPUContext;

struct WaitQueue;

// Task control block
typedef struct task_struct {
    uint32_t pid;
    uint32_t ppid;
    TaskState state;
    TaskPriority priority;
    uint32_t nice;
    
    // Timing information
    uint64_t start_time;
    uint64_t total_runtime;
    uint64_t last_run_time;
    
    // Stack information
    void* stack_base;
    uint64_t stack_size;
    void* stack_top;  // Top of stack (for context switching)
    
    // CPU context
    CPUContext context;
    
    // Scheduling
    uint64_t vruntime;
    uint64_t time_slice;
    uint64_t time_remaining;
    
    // Function to execute
    void (*task_function)(void);
    void* task_data;
    
    // Linked list pointers
    struct task_struct* next;
    struct task_struct* prev;

    /* Per-task translation root; default is boot kernel PML4 (identity map). */
    AddressSpace address_space;

    /* Append-only: keep existing field offsets stable. */
    uint64_t sleep_until_tick;
    struct task_struct* wait_next;
    FileDesc fds[FD_MAX];
    /* Wait queue this task is linked on while BLOCKED (NULL otherwise). */
    struct WaitQueue* wait_queue;
} TaskStruct;

/* Intrusive wait queue (tasks linked via wait_next). */
typedef struct WaitQueue {
    TaskStruct* head;
} WaitQueue;

// Scheduler state
typedef struct {
    TaskStruct* current_task;
    TaskStruct* ready_queue[5];  // One queue per priority level
    uint32_t next_pid;
    bool scheduler_active;
    uint64_t total_tasks;
} SchedulerState;

// Global scheduler state
extern SchedulerState scheduler;

// Function declarations
void scheduler_init(void);
void scheduler_tick(void);
void scheduler_yield(void);
TaskStruct* scheduler_create_task(void (*function)(void), void* data, TaskPriority priority);
/* Kill a task by PID (any state). Kept void for existing callers; see scheduler_kill_task. */
void scheduler_destroy_task(uint32_t pid);
/* Kill a task by PID. Returns false if no live task has that PID (PID 0 is refused).
 * Killing the calling task does not return. */
bool scheduler_kill_task(uint32_t pid);
/* End the calling task (ZOMBIE, reclaimed later). Never returns. */
void scheduler_exit_current(void) __attribute__((noreturn));
void scheduler_schedule(void);
TaskStruct* scheduler_get_current_task(void);
void scheduler_set_priority(uint32_t pid, TaskPriority priority);
void scheduler_print_tasks(void);
void scheduler_kill_all_except_idle(void);
uint32_t scheduler_get_task_count(void);
/* custom_pid must be non-zero and not used by a live task. */
TaskStruct* scheduler_create_task_with_pid(void (*function)(void), void* data, TaskPriority priority, uint32_t custom_pid);
/* After init: idle TCB may carry kmain/shell context; allow tick-driven schedule. */
void scheduler_end_bootstrap(void);

/* Block current task on wq until irq/timer wake; sleep_ms uses tick deadline.
 * Both return only once the task is runnable again (idling in place if nothing else can run). */
void scheduler_block(WaitQueue* wq);
void scheduler_wake_one(WaitQueue* wq);
void scheduler_wake_all(WaitQueue* wq);
void scheduler_sleep_ms(uint32_t ms);
/* Park a ready task onto a wait queue / arm sleep without running its entry (self-test). */
bool scheduler_park(TaskStruct* t, WaitQueue* wq);
bool scheduler_arm_sleep(TaskStruct* t, uint64_t sleep_until_tick);
void scheduler_service_sleepers(void);

// Task management
void task_init(TaskStruct* task, void (*function)(void), void* data, TaskPriority priority);
void task_set_address_space(TaskStruct* task, uint64_t pml4_phys);
uint64_t scheduler_kernel_pml4_phys(void);
/* Switch CR3 + kernel stack from `from` to `to`; returns when `from` is resumed. */
void task_switch(TaskStruct* from, TaskStruct* to);
/* Prepare a new task's stack so its first switch_to() enters task_entry_trampoline. */
void setup_task_context(TaskStruct* task);

// Assembly (context_switch.asm)
/* Save callee-saved regs on the current stack, *old_rsp = RSP, switch to new_rsp. */
void switch_to(uint64_t* old_rsp, uint64_t new_rsp);
void task_entry_trampoline(void);
/* Called by task_entry_trampoline on a new task's first run. Never returns. */
void scheduler_task_entry(void (*fn)(void), void* data) __attribute__((noreturn));

// Stack management
void* task_allocate_stack(uint64_t size);
void task_free_stack(void* stack);

// Idle task
void idle_task(void);

// Debug task function
void debug_task_function(void);

#endif // SCHEDULER_H
