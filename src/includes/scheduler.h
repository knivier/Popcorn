// src/includes/scheduler.h
#ifndef SCHEDULER_H
#define SCHEDULER_H

#include <stdint.h>
#include <stdbool.h>

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

// CPU context structure for x86-64
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
    
    // FPU/SSE state (simplified - just mark as present)
    uint64_t fpu_state[32]; // XMM0-XMM15 registers
    uint64_t fpu_control;
} CPUContext;

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
} TaskStruct;

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
void scheduler_destroy_task(uint32_t pid);
void scheduler_schedule(void);
TaskStruct* scheduler_get_current_task(void);
void scheduler_set_priority(uint32_t pid, TaskPriority priority);

// Task management
void task_init(TaskStruct* task, void (*function)(void), void* data, TaskPriority priority);
void task_switch(TaskStruct* from, TaskStruct* to);
void task_exit(void);
void setup_task_context(TaskStruct* task);

// Assembly functions for context switching
void context_save(CPUContext* context);
void context_restore(CPUContext* context);
void context_switch_to_task(TaskStruct* task);

// Stack management
void* task_allocate_stack(uint64_t size);
void task_free_stack(void* stack);

// Idle task
void idle_task(void);

#endif // SCHEDULER_H
