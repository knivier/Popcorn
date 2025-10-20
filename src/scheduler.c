// src/scheduler.c
#include "includes/scheduler.h"
#include "includes/timer.h"
#include "includes/console.h"
#include <stddef.h>

// Global scheduler state
SchedulerState scheduler = {0};

// External functions
extern uint64_t timer_get_ticks(void);

// Initialize the scheduler
void scheduler_init(void) {
    // Initialize scheduler state
    scheduler.current_task = NULL;
    scheduler.next_pid = 1;
    scheduler.scheduler_active = false;
    scheduler.total_tasks = 0;
    
    // Initialize ready queues
    for (int i = 0; i < 5; i++) {
        scheduler.ready_queue[i] = NULL;
    }
    
    // Create idle task
    TaskStruct* idle = scheduler_create_task(idle_task, NULL, PRIORITY_IDLE);
    if (idle) {
        idle->pid = 0;  // Special PID for idle task
        scheduler.current_task = idle;
        scheduler.current_task->state = TASK_STATE_RUNNING;
    }
    
    scheduler.scheduler_active = true;
    console_println_color("Scheduler initialized", CONSOLE_SUCCESS_COLOR);
}

// Scheduler tick handler (called from timer interrupt)
void scheduler_tick(void) {
    if (!scheduler.scheduler_active || !scheduler.current_task) {
        return;
    }
    
    // Update current task runtime
    uint64_t current_time = timer_get_ticks();
    scheduler.current_task->total_runtime += 
        current_time - scheduler.current_task->last_run_time;
    scheduler.current_task->last_run_time = current_time;
    
    // Decrease time slice
    if (scheduler.current_task->time_remaining > 0) {
        scheduler.current_task->time_remaining--;
    }
    
    // Check if task should be preempted
    if (scheduler.current_task->time_remaining == 0) {
        scheduler_schedule();
    }
}

// Yield CPU to another task
void scheduler_yield(void) {
    if (scheduler.scheduler_active) {
        scheduler_schedule();
    }
}

// Create a new task
TaskStruct* scheduler_create_task(void (*function)(void), void* data, TaskPriority priority) {
    if (!function) {
        return NULL;
    }
    
    // Allocate task structure (simplified - in real system would use kmalloc)
    static TaskStruct task_pool[32];
    static uint32_t task_pool_index = 0;
    
    if (task_pool_index >= 32) {
        console_println_color("Task pool exhausted", CONSOLE_ERROR_COLOR);
        return NULL;
    }
    
    TaskStruct* task = &task_pool[task_pool_index++];
    
    // Initialize task
    task_init(task, function, data, priority);
    task->pid = scheduler.next_pid++;
    task->start_time = timer_get_ticks();
    task->last_run_time = task->start_time;
    
    // Add to ready queue
    task->next = scheduler.ready_queue[priority];
    if (scheduler.ready_queue[priority]) {
        scheduler.ready_queue[priority]->prev = task;
    }
    scheduler.ready_queue[priority] = task;
    task->prev = NULL;
    
    scheduler.total_tasks++;
    return task;
}

// Destroy a task
void scheduler_destroy_task(uint32_t pid) {
    // Find task in all queues
    TaskStruct* task = NULL;
    TaskPriority priority;
    
    for (priority = PRIORITY_IDLE; priority <= PRIORITY_REALTIME; priority++) {
        TaskStruct* current = scheduler.ready_queue[priority];
        while (current) {
            if (current->pid == pid) {
                task = current;
                break;
            }
            current = current->next;
        }
        if (task) break;
    }
    
    if (!task) {
        return;
    }
    
    // Remove from queue
    if (task->prev) {
        task->prev->next = task->next;
    } else {
        scheduler.ready_queue[priority] = task->next;
    }
    
    if (task->next) {
        task->next->prev = task->prev;
    }
    
    // Mark as zombie
    task->state = TASK_STATE_ZOMBIE;
    scheduler.total_tasks--;
}

// Main scheduling function
void scheduler_schedule(void) {
    if (!scheduler.scheduler_active) {
        return;
    }
    
    TaskStruct* next_task = NULL;
    
    // Find highest priority ready task
    for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
        if (scheduler.ready_queue[priority]) {
            next_task = scheduler.ready_queue[priority];
            break;
        }
    }
    
    // If no ready task, use idle task
    if (!next_task) {
        next_task = scheduler.current_task;  // Keep running current task
    }
    
    // Switch to next task if different
    if (next_task != scheduler.current_task) {
        TaskStruct* old_task = scheduler.current_task;
        scheduler.current_task = next_task;
        
        // Update task states
        if (old_task && old_task->state == TASK_STATE_RUNNING) {
            old_task->state = TASK_STATE_READY;
        }
        
        scheduler.current_task->state = TASK_STATE_RUNNING;
        scheduler.current_task->time_remaining = scheduler.current_task->time_slice;
        
        // Perform context switch (simplified)
        task_switch(old_task, scheduler.current_task);
    }
}

// Get current running task
TaskStruct* scheduler_get_current_task(void) {
    return scheduler.current_task;
}

// Set task priority
void scheduler_set_priority(uint32_t pid, TaskPriority priority) {
    // Find and update task priority
    for (int p = PRIORITY_IDLE; p <= PRIORITY_REALTIME; p++) {
        TaskStruct* current = scheduler.ready_queue[p];
        while (current) {
            if (current->pid == pid) {
                current->priority = priority;
                // Note: In a real system, would need to move task between queues
                return;
            }
            current = current->next;
        }
    }
}

// Initialize a task structure
void task_init(TaskStruct* task, void (*function)(void), void* data, TaskPriority priority) {
    if (!task) return;
    
    task->pid = 0;
    task->ppid = 0;
    task->state = TASK_STATE_READY;
    task->priority = priority;
    task->nice = 0;
    
    task->start_time = 0;
    task->total_runtime = 0;
    task->last_run_time = 0;
    
    task->stack_base = NULL;
    task->stack_size = 0;
    
    task->vruntime = 0;
    task->time_slice = 10;  // 10 ticks default
    task->time_remaining = task->time_slice;
    
    task->task_function = function;
    task->task_data = data;
    
    task->next = NULL;
    task->prev = NULL;
}

// Context switch (simplified - no actual register saving)
void task_switch(TaskStruct* from, TaskStruct* to) {
    // In a real system, this would save/restore CPU registers
    // For now, just update the current task pointer
    (void)from;  // Suppress unused parameter warning
    (void)to;    // Suppress unused parameter warning
}

// Task exit
void task_exit(void) {
    TaskStruct* current = scheduler_get_current_task();
    if (current) {
        scheduler_destroy_task(current->pid);
    }
}

// Idle task - runs when no other tasks are ready
void idle_task(void) {
    while (1) {
        // In a real system, this would halt the CPU or enter power saving
        __asm__ volatile("hlt");
    }
}
