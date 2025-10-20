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

// Idle task
void idle_task(void);

#endif // SCHEDULER_H
