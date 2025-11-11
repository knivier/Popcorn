// src/core/scheduler.c
#include "../includes/scheduler.h"
#include "../includes/timer.h"
#include "../includes/console.h"
#include "../includes/memory.h"
#include "../includes/utils.h"
#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

// Global scheduler state
SchedulerState scheduler = {0};

// External functions
extern uint64_t timer_get_ticks(void);
extern unsigned char read_port(unsigned short port);
extern void write_port(unsigned short port, unsigned char data);

// Stack management constants
#define TASK_STACK_SIZE (16 * 1024)  // 16KB per task stack
#define STACK_ALIGNMENT 16           // 16-byte alignment for x86-64

// Serial port for debugging (COM1)
#define SERIAL_PORT 0x3F8

static void serial_putc(char c) {
    // Wait for serial port to be ready
    while ((read_port(SERIAL_PORT + 5) & 0x20) == 0);
    write_port(SERIAL_PORT, c);
}

static void serial_print(const char* str) {
    while (*str) {
        serial_putc(*str++);
    }
}

// Stack management functions
void* task_allocate_stack(uint64_t size) {
    (void)size;  // Suppress unused parameter warning
    
    // Use static stack allocation to avoid memory manager issues during early boot
    static char static_stacks[32][16384];  // 32 tasks, 16KB each
    static uint32_t stack_index = 0;
    
    if (stack_index >= 32) {
        return NULL;
    }
    
    void* stack = static_stacks[stack_index++];
    
    // Clear the stack
    memset(stack, 0, 16384);
    
    return stack;
}

void task_free_stack(void* stack) {
    // For static allocation, we don't need to free anything
    // Just mark as unused (simplified for debugging)
    (void)stack;  // Suppress unused parameter warning
}

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
    } else {
        serial_print("ERROR: Failed to create idle task\n");
    }

    scheduler.scheduler_active = true;
    console_println_color("Scheduler initialized", CONSOLE_SUCCESS_COLOR);
}

// Scheduler tick handler (called from timer interrupt)
void scheduler_tick(void) {
    // Disable scheduler tick during early boot to avoid crashes
    static bool first_tick = true;
    if (first_tick) {
        first_tick = false;
        return;  // Skip first tick to avoid early crashes
    }

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

    // Force preemption every few ticks to ensure responsiveness
    static int tick_counter = 0;
    tick_counter++;
    
    // Preempt every 10 ticks (much more frequent) regardless of time slice
    if (tick_counter >= 10) {
        tick_counter = 0;
        scheduler_schedule();
        return;
    }

    // Check if task should be preempted (only if we have multiple tasks)
    if (scheduler.current_task->time_remaining == 0 && scheduler.total_tasks > 1) {
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
        serial_print("ERROR: No function provided for task creation\n");
        return NULL;
    }

    // Allocate task structure (simplified - in real system would use kmalloc)
    static TaskStruct task_pool[32];
    static uint32_t task_pool_index = 0;

    if (task_pool_index >= 32) {
        console_println_color("Task pool exhausted", CONSOLE_ERROR_COLOR);
        serial_print("ERROR: Task pool exhausted\n");
        return NULL;
    }

    TaskStruct* task = &task_pool[task_pool_index++];

    // Initialize task
    task_init(task, function, data, priority);
    task->pid = scheduler.next_pid++;
    task->start_time = timer_get_ticks();
    task->last_run_time = task->start_time;

    // Allocate and set up stack
    task->stack_size = TASK_STACK_SIZE;
    task->stack_base = task_allocate_stack(task->stack_size);
    if (!task->stack_base) {
        console_println_color("Failed to allocate task stack", CONSOLE_ERROR_COLOR);
        serial_print("ERROR: Failed to allocate task stack\n");
        return NULL;
    }

    // Set stack top (stack grows downward, ensure 16-byte alignment)
    uintptr_t stack_top_addr = (uintptr_t)task->stack_base + task->stack_size;
    // Align to 16 bytes (x86-64 ABI requirement)
    stack_top_addr = stack_top_addr & ~((uintptr_t)15);
    task->stack_top = (void*)stack_top_addr;

    // Set up initial context for the task
    setup_task_context(task);

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
    for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
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
        // Find which queue this task is in
        for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
            if (scheduler.ready_queue[priority] == task) {
                scheduler.ready_queue[priority] = task->next;
                break;
            }
        }
    }
    if (task->next) {
        task->next->prev = task->prev;
    }

    // Free stack
    task_free_stack(task->stack_base);

    // Mark as zombie
    task->state = TASK_STATE_ZOMBIE;
    scheduler.total_tasks--;
}

// Main scheduling function
void scheduler_schedule(void) {
    if (!scheduler.scheduler_active || !scheduler.current_task) {
        return;
    }

    // Clean up zombie tasks first
    for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
        TaskStruct* task = scheduler.ready_queue[priority];
        while (task) {
            TaskStruct* next = task->next;
            if (task->state == TASK_STATE_ZOMBIE) {
                // Remove from queue
                if (task->prev) {
                    task->prev->next = task->next;
                } else {
                    scheduler.ready_queue[priority] = task->next;
                }
                if (task->next) {
                    task->next->prev = task->prev;
                }
                
                // Update task count
                scheduler.total_tasks--;
                
                // If this was the current task, switch to idle
                if (task == scheduler.current_task) {
                    scheduler.current_task = NULL;  // Will be set to idle below
                }
            }
            task = next;
        }
    }

    TaskStruct* next_task = NULL;

    // Find next task with proper multi-level scheduling
    // First, try to find a different task in the same priority
    if (scheduler.current_task && scheduler.current_task->priority >= PRIORITY_IDLE && 
        scheduler.current_task->priority <= PRIORITY_REALTIME) {
        
        int current_priority = scheduler.current_task->priority;
        if (scheduler.ready_queue[current_priority]) {
            // Try to find next task in same priority
            next_task = scheduler.current_task->next;
            if (!next_task) {
                // Wrap around to beginning of queue
                next_task = scheduler.ready_queue[current_priority];
            }
            
            // If we found the same task, look for tasks in other priorities
            if (next_task == scheduler.current_task) {
                // Look for tasks in other priority levels
                for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
                    if (priority != current_priority && scheduler.ready_queue[priority]) {
                        next_task = scheduler.ready_queue[priority];
                        break;
                    }
                }
            }
        }
    } else {
        // No current task, find highest priority task
        for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
            if (scheduler.ready_queue[priority]) {
                next_task = scheduler.ready_queue[priority];
                break;
            }
        }
    }

    // If no ready task, use idle task (or current task if it's the idle task)
    if (!next_task) {
        // If current task is idle or no other tasks, just return
        if (scheduler.current_task && scheduler.current_task->pid == 0) {
            return;
        }
        next_task = scheduler.current_task;  // Keep running current task
    }
    
    // Fallback: if no current task, find the idle task
    if (!scheduler.current_task) {
        // Find idle task (PID 0)
        for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
            TaskStruct* task = scheduler.ready_queue[priority];
            while (task) {
                if (task->pid == 0) {
                    scheduler.current_task = task;
                    break;
                }
                task = task->next;
            }
            if (scheduler.current_task) break;
        }
    }

    // Switch to next task if different
    if (next_task != scheduler.current_task) {
        TaskStruct* old_task = scheduler.current_task;

        // Update task states
        if (old_task && old_task->state == TASK_STATE_RUNNING) {
            old_task->state = TASK_STATE_READY;
        }

        next_task->state = TASK_STATE_RUNNING;
        next_task->time_remaining = next_task->time_slice;

        // Perform context switch only if we have a valid context
        if (next_task->stack_base && next_task->context.rip) {
            // Additional safety checks
            if (next_task->context.rip < 0x1000) {
                return;
            }
            if (next_task->context.rsp < 0x1000) {
                return;
            }
            
            task_switch(old_task, next_task);
        } else {
            serial_print("ERROR: Invalid task context for switching\n");
        }
    } else {
        // Same task, no switch needed
    }
}

// Get current running task
TaskStruct* scheduler_get_current_task(void) {
    return scheduler.current_task;
}

// Get total number of tasks
uint32_t scheduler_get_task_count(void) {
    return scheduler.total_tasks;
}

// Print all tasks
void scheduler_print_tasks(void) {
    console_println_color("PID | State    | Priority | Runtime", CONSOLE_FG_COLOR);
    console_println_color("----|----------|----------|--------", CONSOLE_FG_COLOR);
    
    // Print idle task first
    if (scheduler.current_task && scheduler.current_task->pid == 0) {
        console_print_color("0   | Running  | Idle     | ", CONSOLE_FG_COLOR);
        char buffer[32];
        int_to_str((int)scheduler.current_task->total_runtime, buffer);
        console_println_color(buffer, CONSOLE_FG_COLOR);
    }
    
    // Print all other tasks
    for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
        TaskStruct* task = scheduler.ready_queue[priority];
        while (task) {
            if (task->pid != 0) {  // Skip idle task (already printed)
                char buffer[32];
                int_to_str(task->pid, buffer);
                console_print_color(buffer, CONSOLE_FG_COLOR);
                console_print_color("   | ", CONSOLE_FG_COLOR);
                
                // Print state
                switch (task->state) {
                    case TASK_STATE_READY:
                        console_print_color("Ready    | ", CONSOLE_FG_COLOR);
                        break;
                    case TASK_STATE_RUNNING:
                        console_print_color("Running  | ", CONSOLE_FG_COLOR);
                        break;
                    case TASK_STATE_BLOCKED:
                        console_print_color("Blocked  | ", CONSOLE_FG_COLOR);
                        break;
                    case TASK_STATE_SLEEPING:
                        console_print_color("Sleeping | ", CONSOLE_FG_COLOR);
                        break;
                    case TASK_STATE_ZOMBIE:
                        console_print_color("Zombie   | ", CONSOLE_FG_COLOR);
                        break;
                    default:
                        console_print_color("Unknown  | ", CONSOLE_FG_COLOR);
                        break;
                }
                
                // Print priority
                switch (task->priority) {
                    case PRIORITY_IDLE:
                        console_print_color("Idle     | ", CONSOLE_FG_COLOR);
                        break;
                    case PRIORITY_LOW:
                        console_print_color("Low      | ", CONSOLE_FG_COLOR);
                        break;
                    case PRIORITY_NORMAL:
                        console_print_color("Normal   | ", CONSOLE_FG_COLOR);
                        break;
                    case PRIORITY_HIGH:
                        console_print_color("High     | ", CONSOLE_FG_COLOR);
                        break;
                    case PRIORITY_REALTIME:
                        console_print_color("Realtime | ", CONSOLE_FG_COLOR);
                        break;
                    default:
                        console_print_color("Unknown  | ", CONSOLE_FG_COLOR);
                        break;
                }
                
                // Print runtime
                int_to_str(task->total_runtime, buffer);
                console_println_color(buffer, CONSOLE_FG_COLOR);
            }
            task = task->next;
        }
    }
}

// Set task priority
void scheduler_set_priority(uint32_t pid, TaskPriority priority) {
    // Find and update task priority
    for (int p = PRIORITY_IDLE; p <= PRIORITY_REALTIME; p++) {
        TaskStruct* current = scheduler.ready_queue[p];
        while (current) {
            if (current->pid == pid) {
                current->priority = priority;
                return;
            }
            current = current->next;
        }
    }
}

// Initialize a task structure
void task_init(TaskStruct* task, void (*function)(void), void* data, TaskPriority priority) {
    if (!task || !function) return;
    
    task->pid = 0;  // Will be set by caller
    task->ppid = 0;
    task->state = TASK_STATE_READY;
    task->priority = priority;
    task->nice = 0;
    
    task->start_time = 0;  // Will be set by caller
    task->total_runtime = 0;
    task->last_run_time = 0;
    
    task->stack_base = NULL;  // Will be set by caller
    task->stack_size = 0;
    task->stack_top = NULL;
    
    // Initialize context
    memset(&task->context, 0, sizeof(task->context));
    
    task->vruntime = 0;
    task->time_slice = 100;  // Default time slice
    task->time_remaining = task->time_slice;
    
    task->task_function = function;
    task->task_data = data;
    
    task->next = NULL;
    task->prev = NULL;
}

// Set up initial context for a new task
void setup_task_context(TaskStruct* task) {
    if (!task || !task->stack_top) {
        serial_print("ERROR: Invalid task or stack_top in setup_task_context\n");
        return;
    }

    // Ensure stack bounds are valid
    if ((uintptr_t)task->stack_top <= (uintptr_t)task->stack_base) {
        serial_print("ERROR: Stack top <= stack base\n");
        return;
    }

    // Calculate safe stack pointer with bounds checking
    uintptr_t stack_size = (uintptr_t)task->stack_top - (uintptr_t)task->stack_base;
    if (stack_size < 256) {  // Minimum 256 bytes for safety
        serial_print("ERROR: Stack too small\n");
        return;
    }

    // Pre-fill the stack with iretq frame
    // iretq expects: SS, RSP, RFLAGS, CS, RIP (in that order, bottom to top)
    uintptr_t stack_ptr = (uintptr_t)task->stack_top;
    
    // Reserve space for iretq frame (5 * 8 bytes = 40 bytes)
    stack_ptr -= 40;
    
    // Ensure 16-byte alignment for x86-64 ABI
    stack_ptr = stack_ptr & ~((uintptr_t)15);

    // Bounds check - ensure we don't go below stack base
    if (stack_ptr < (uintptr_t)task->stack_base + 64) {
        serial_print("ERROR: Stack pointer too low\n");
        stack_ptr = (uintptr_t)task->stack_base + 64;
        stack_ptr = stack_ptr & ~((uintptr_t)15);  // Re-align
    }

    // Pre-fill the iretq frame on the stack
    uint64_t* stack_frame = (uint64_t*)stack_ptr;
    stack_frame[0] = (uint64_t)task->task_function;  // RIP
    stack_frame[1] = 0x08;  // CS (kernel code segment)
    stack_frame[2] = 0x202;  // RFLAGS (IF=1, bit 1 always 1)
    stack_frame[3] = stack_ptr;  // RSP (points to this frame)
    stack_frame[4] = 0x10;  // SS (kernel data segment)

    task->context.rsp = stack_ptr;
    
    // Validate RSP
    if (task->context.rsp == 0) {
        serial_print("ERROR: NULL RSP in task context\n");
        return;
    }
    if (task->context.rsp < 0x1000) {
        serial_print("ERROR: RSP too low (< 0x1000)\n");
        return;
    }

    // Set up segment registers for long mode
    task->context.cs = 0x08;  
    task->context.ss = 0x10;  
    task->context.ds = 0x10;
    task->context.es = 0x10;
    task->context.fs = 0x10;
    task->context.gs = 0x10;

    // Set up flags register (interrupts enabled, bit 1 must be 1)
    task->context.rflags = 0x202;  // IF=1, bit 1=1 (required)

    // Set up function arguments
    task->context.rdi = (uint64_t)task->task_data;  // First argument

    // Clear other registers
    task->context.rax = 0;
    task->context.rbx = 0;
    task->context.rcx = 0;
    task->context.rdx = 0;
    task->context.rsi = 0;
    task->context.rbp = 0;
    task->context.r8 = 0;
    task->context.r9 = 0;
    task->context.r10 = 0;
    task->context.r11 = 0;
    task->context.r12 = 0;
    task->context.r13 = 0;
    task->context.r14 = 0;
    task->context.r15 = 0;

    // Set up FPU state
    task->context.fpu_control = 0x37F;  // Default FPU control word
    memset(task->context.fpu_state, 0, sizeof(task->context.fpu_state));
}

// Context switch - now with real CPU register saving/restoring
void task_switch(TaskStruct* from, TaskStruct* to) {
    if (!to) return;

    // Disable interrupts during context switch
    __asm__ volatile("cli");

    // If we have a current task, save its context
    if (from && from != to) {
        context_save(&from->context);
    }

    // Switch to the new task
    scheduler.current_task = to;

    // Restore the new task's context
    context_restore(&to->context);

    // This should never return, as context_restore executes iretq
    // If we get here, something went wrong
    __asm__ volatile("sti");
}

// Task exit
void task_exit(void) {
    TaskStruct* current = scheduler_get_current_task();
    if (current) {
        current->state = TASK_STATE_ZOMBIE;
    }
}

// Idle task - runs when no other tasks are ready
void idle_task(void) {
    // Very simple idle loop - just increment a counter
    static int counter = 0;
    while (1) {
        counter++;
    }
}

// Test task function to demonstrate context switching
void test_task_function(void) {
    static int counter = 0;

    while (1) {
        counter++;

        if (counter % 10 == 0) {
            scheduler_yield();
        }

        for (volatile int i = 0; i < 10; i++);
    }
}

// Debug task function for mon -debug command
void debug_task_function(void) {
    static int counter = 0;

    while (1) {
        counter++;

        if (counter % 10 == 0) {
            scheduler_yield();
        }

        for (volatile int i = 0; i < 10; i++);
    }
}

// Create a task with a specific PID
TaskStruct* scheduler_create_task_with_pid(void (*function)(void), void* data, TaskPriority priority, uint32_t custom_pid) {
    if (!function) {
        serial_print("ERROR: No function provided for task creation\n");
        return NULL;
    }

    // Allocate task structure (simplified - in real system would use kmalloc)
    static TaskStruct task_pool[32];
    static uint32_t task_pool_index = 0;

    if (task_pool_index >= 32) {
        console_println_color("Task pool exhausted", CONSOLE_ERROR_COLOR);
        serial_print("ERROR: Task pool exhausted\n");
        return NULL;
    }

    TaskStruct* task = &task_pool[task_pool_index++];

    // Initialize task
    task_init(task, function, data, priority);
    task->pid = custom_pid;  // Use custom PID
    task->start_time = timer_get_ticks();
    task->last_run_time = task->start_time;

    // Allocate and set up stack
    task->stack_size = TASK_STACK_SIZE;
    task->stack_base = task_allocate_stack(task->stack_size);
    if (!task->stack_base) {
        console_println_color("Failed to allocate task stack", CONSOLE_ERROR_COLOR);
        serial_print("ERROR: Failed to allocate task stack\n");
        return NULL;
    }

    // Set stack top (stack grows downward, ensure 16-byte alignment)
    uintptr_t stack_top_addr = (uintptr_t)task->stack_base + task->stack_size;
    // Align to 16 bytes (x86-64 ABI requirement)
    stack_top_addr = stack_top_addr & ~((uintptr_t)15);
    task->stack_top = (void*)stack_top_addr;

    // Set up initial context for the task
    setup_task_context(task);

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

// Kill all tasks except the idle task (PID 0)
void scheduler_kill_all_except_idle(void) {
    for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
        TaskStruct* task = scheduler.ready_queue[priority];
        while (task) {
            TaskStruct* next = task->next;
            if (task->pid != 0) {  // Don't kill idle task
                // Remove from queue
                if (task->prev) {
                    task->prev->next = task->next;
                } else {
                    scheduler.ready_queue[priority] = task->next;
                }
                if (task->next) {
                    task->next->prev = task->prev;
                }
                
                // Update task count
                scheduler.total_tasks--;
                
                // If this was the current task, switch to idle
                if (task == scheduler.current_task) {
                    scheduler.current_task = NULL;  // Will be set to idle below
                }
            }
            task = next;
        }
    }
    
    // Ensure idle task is running
    if (!scheduler.current_task) {
        // Find idle task (PID 0)
        for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
            TaskStruct* task = scheduler.ready_queue[priority];
            while (task) {
                if (task->pid == 0) {
                    scheduler.current_task = task;
                    task->state = TASK_STATE_RUNNING;
                    break;
                }
                task = task->next;
            }
            if (scheduler.current_task) break;
        }
    }
}