// src/core/scheduler.c
#include "../includes/scheduler.h"
#include "../includes/timer.h"
#include "../includes/console.h"
#include "../includes/memory.h"
#include "../includes/utils.h"
#include "../includes/device.h"
#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

extern char __kernel_start[];
extern char __text_vend[];

_Static_assert(offsetof(TaskStruct, context) == 72, "TaskStruct.context offset");
_Static_assert(offsetof(CPUContext, r15) == 0, "asm context_save: update offsets");
_Static_assert(offsetof(CPUContext, rax) == 112, "asm context_save: update offsets");
_Static_assert(offsetof(CPUContext, rip) == 120, "asm context_save: mov [r10+120]");
_Static_assert(offsetof(CPUContext, rsp) == 128, "asm context_save/restore: +128");
_Static_assert(offsetof(CPUContext, rflags) == 136, "asm context_save: rflags");
_Static_assert(offsetof(TaskStruct, address_space) == 584, "task_switch vmm: reload offsetof");

static bool context_looks_sane(const CPUContext* c) {
    if (!c) {
        return false;
    }
    const uintptr_t rip = (uintptr_t)c->rip;
    const uintptr_t sp = (uintptr_t)c->rsp;
    const uintptr_t t0 = (uintptr_t)__kernel_start;
    const uintptr_t t1 = (uintptr_t)__text_vend;
    if (rip < t0 || rip >= t1) {
        return false;
    }
    if (sp < 0x1000U || (sp & 7U) != 0U) {
        return false;
    }
    if (sp < 24U) {
        return false;
    }
    return true;
}

// Global scheduler state
SchedulerState scheduler = {0};

/* Boot page-table root (asm identity map); all tasks use this until given another PML4. */
static uint64_t g_kernel_pml4_phys;

/* current_task may be the idle task while the CPU still runs kmain on the boot stack. Saving
 * that "idle" context would clobber setup_task_context() with kmain's RIP/RSP and fault on iretq. */
static bool idle_cpu_has_run;

/* True while scheduler.current_task is synthetic idle but CPU still executes kmain boot stack. */
static bool bootstrap_on_kmain_stack(void) {
    return scheduler.current_task &&
           scheduler.current_task->pid == 0 &&
           !idle_cpu_has_run;
}

#define TASK_POOL_CAP 32u
static TaskStruct g_task_pool[TASK_POOL_CAP];
static uint32_t g_task_pool_index;

void scheduler_end_bootstrap(void) {
    /*
     * Entering the interactive shell: allow ticks/yields. The idle TCB becomes the
     * carrier for kmain's context on the first real preempt (see task_switch fake_idle).
     */
    idle_cpu_has_run = true;
}

static void ready_remove(TaskStruct* task) {
    if (!task) {
        return;
    }
    int p = (int)task->priority;
    if (p < PRIORITY_IDLE || p > PRIORITY_REALTIME) {
        return;
    }
    if (task->prev) {
        task->prev->next = task->next;
    } else if (scheduler.ready_queue[p] == task) {
        scheduler.ready_queue[p] = task->next;
    }
    if (task->next) {
        task->next->prev = task->prev;
    }
    task->next = NULL;
    task->prev = NULL;
}

static void ready_add(TaskStruct* task) {
    if (!task) {
        return;
    }
    int p = (int)task->priority;
    if (p < PRIORITY_IDLE || p > PRIORITY_REALTIME) {
        return;
    }
    task->next = scheduler.ready_queue[p];
    if (scheduler.ready_queue[p]) {
        scheduler.ready_queue[p]->prev = task;
    }
    scheduler.ready_queue[p] = task;
    task->prev = NULL;
}

static void wake_expired_sleepers(void) {
    uint64_t now = timer_get_ticks();
    for (uint32_t i = 0; i < g_task_pool_index; i++) {
        TaskStruct* t = &g_task_pool[i];
        if (t->state == TASK_STATE_SLEEPING && now >= t->sleep_until_tick) {
            t->state = TASK_STATE_READY;
            ready_add(t);
        }
    }
}

void scheduler_block(WaitQueue* wq) {
    TaskStruct* t = scheduler.current_task;
    if (!t || !wq || bootstrap_on_kmain_stack()) {
        return;
    }
    ready_remove(t);
    t->state = TASK_STATE_BLOCKED;
    t->wait_next = wq->head;
    wq->head = t;
    scheduler_schedule();
}

void scheduler_wake_one(WaitQueue* wq) {
    if (!wq || !wq->head) {
        return;
    }
    TaskStruct* t = wq->head;
    wq->head = t->wait_next;
    t->wait_next = NULL;
    if (t->state == TASK_STATE_BLOCKED) {
        t->state = TASK_STATE_READY;
        ready_add(t);
    }
}

void scheduler_wake_all(WaitQueue* wq) {
    if (!wq) {
        return;
    }
    while (wq->head) {
        scheduler_wake_one(wq);
    }
}

void scheduler_sleep_ms(uint32_t ms) {
    TaskStruct* t = scheduler.current_task;
    if (!t || bootstrap_on_kmain_stack()) {
        return;
    }
    if (ms == 0) {
        scheduler_yield();
        return;
    }
    ready_remove(t);
    t->sleep_until_tick = timer_get_ticks() + timer_ms_to_ticks(ms);
    t->state = TASK_STATE_SLEEPING;
    t->wait_next = NULL;
    scheduler_schedule();
}

bool scheduler_park(TaskStruct* t, WaitQueue* wq) {
    if (!t || !wq) {
        return false;
    }
    ready_remove(t);
    t->state = TASK_STATE_BLOCKED;
    t->wait_next = wq->head;
    wq->head = t;
    return true;
}

bool scheduler_arm_sleep(TaskStruct* t, uint64_t sleep_until_tick) {
    if (!t) {
        return false;
    }
    ready_remove(t);
    t->sleep_until_tick = sleep_until_tick;
    t->state = TASK_STATE_SLEEPING;
    t->wait_next = NULL;
    return true;
}

void scheduler_service_sleepers(void) {
    wake_expired_sleepers();
}

// External functions
extern uint64_t timer_get_ticks(void);
extern void boot_serial_putc(char c);

// Stack management constants
#define TASK_STACK_SIZE (16 * 1024)  // 16KB per task stack
#define STACK_ALIGNMENT 16           // 16-byte alignment for x86-64

static void serial_print(const char* str) {
    while (str && *str) {
        boot_serial_putc(*str++);
    }
}

static TaskStruct* task_pool_alloc(void) {
    if (g_task_pool_index >= TASK_POOL_CAP) {
        console_println_color("Task pool exhausted", CONSOLE_ERROR_COLOR);
        serial_print("ERROR: Task pool exhausted\n");
        return NULL;
    }
    return &g_task_pool[g_task_pool_index++];
}

// Stack management functions
void* task_allocate_stack(uint64_t size) {
    (void)size;  // Suppress unused parameter warning
    
    // Use static stack allocation to avoid memory manager issues during early boot
    static char static_stacks[TASK_POOL_CAP][16384];  // 32 tasks, 16KB each
    static uint32_t stack_index = 0;
    
    if (stack_index >= TASK_POOL_CAP) {
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
/* Attach a private PML4 (shared identity + kernel half) for non-idle tasks. */
static int task_attach_private_as(TaskStruct* task) {
    uint64_t pml4;
    if (!task) {
        return -1;
    }
    pml4 = vmm_alloc_pml4();
    if (pml4 == 0) {
        serial_print("ERROR: vmm_alloc_pml4 failed\n");
        return -1;
    }
    if (vmm_init_process_address_space(pml4, g_kernel_pml4_phys) != 0) {
        vmm_free_pml4(pml4, g_kernel_pml4_phys);
        serial_print("ERROR: vmm_init_process_address_space failed\n");
        return -1;
    }
    task->address_space.pml4_phys = pml4;
    return 0;
}

void scheduler_init(void) {
    g_kernel_pml4_phys = vmm_get_cr3() & VMM_PTE_ADDR_MASK;

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
    idle_cpu_has_run = false;
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
    console_println_color("  Address spaces: per-task PML4; CR3 on switch", CONSOLE_INFO_COLOR);
}

uint64_t scheduler_kernel_pml4_phys(void) { return g_kernel_pml4_phys; }

void task_set_address_space(TaskStruct* task, uint64_t pml4_phys) {
    if (!task) {
        return;
    }
    /* Caller supplies a root that already shares identity + kernel half. */
    task->address_space.pml4_phys = pml4_phys & VMM_PTE_ADDR_MASK;
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

    wake_expired_sleepers();

    /* Until scheduler_end_bootstrap(), skip preempt — still on kmain boot stack. */
    if (bootstrap_on_kmain_stack()) {
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
    if (scheduler.scheduler_active && !bootstrap_on_kmain_stack()) {
        scheduler_schedule();
    }
}

// Create a new task
TaskStruct* scheduler_create_task(void (*function)(void), void* data, TaskPriority priority) {
    if (!function) {
        serial_print("ERROR: No function provided for task creation\n");
        return NULL;
    }

    TaskStruct* task = task_pool_alloc();
    if (!task) {
        return NULL;
    }

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

    /* Idle keeps the boot master CR3; everyone else gets a private PML4. */
    if (priority != PRIORITY_IDLE) {
        if (task_attach_private_as(task) != 0) {
            task_free_stack(task->stack_base);
            task->stack_base = NULL;
            return NULL;
        }
    }

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

    // Free stack + private PML4 (shared tables stay with the master).
    task_free_stack(task->stack_base);
    if (task->address_space.pml4_phys != 0 &&
        task->address_space.pml4_phys != g_kernel_pml4_phys) {
        vmm_free_pml4(task->address_space.pml4_phys, g_kernel_pml4_phys);
        task->address_space.pml4_phys = g_kernel_pml4_phys;
    }

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

    /* If still unset (e.g. idle queue empty while NORMAL tasks exist), scan all queues. */
    if (!next_task) {
        for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
            TaskStruct* cand = scheduler.ready_queue[priority];
            while (cand) {
                if (cand->state == TASK_STATE_READY || cand->state == TASK_STATE_RUNNING) {
                    if (cand != scheduler.current_task || scheduler.total_tasks <= 1) {
                        next_task = cand;
                        break;
                    }
                }
                cand = cand->next;
            }
            if (next_task && next_task != scheduler.current_task) {
                break;
            }
        }
    }

    if (!next_task) {
        next_task = scheduler.current_task;
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

    task->address_space.pml4_phys = g_kernel_pml4_phys;
    task->sleep_until_tick = 0;
    task->wait_next = NULL;
    fd_table_init(task->fds);
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

    task->context.rip = (uint64_t)task->task_function;
}

// Context switch - now with real CPU register saving/restoring
void task_switch(TaskStruct* from, TaskStruct* to) {
    if (!to) return;

    /* IF=0: pending IRQs (e.g. keyboard) wait; keep this section short to bound input latency. */
    __asm__ volatile("cli");

    // If we have a current task, save its context
    if (from && from != to) {
        const bool fake_idle =
            (from->pid == 0 && !idle_cpu_has_run);
        if (!fake_idle) {
            context_save(&from->context);
            if (!context_looks_sane(&from->context)) {
                serial_print("FATAL: context_save produced invalid RIP/RSP; halting\n");
                for (;;) {
                    __asm__ volatile("cli; hlt");
                }
            }
        }
    }

    /* Switch address space after saving outgoing state (identity + kernel shared). */
    if (to->address_space.pml4_phys != 0) {
        uint64_t want = to->address_space.pml4_phys & VMM_PTE_ADDR_MASK;
        uint64_t cur = vmm_get_cr3() & VMM_PTE_ADDR_MASK;
        if (want != cur) {
            vmm_load_cr3(want);
        }
    }

    // Switch to the new task
    scheduler.current_task = to;

    if (!context_looks_sane(&to->context)) {
        serial_print("FATAL: would iretq to non-text RIP or bad RSP; halting\n");
        for (;;) {
            __asm__ volatile("cli; hlt");
        }
    }

    // Restore the new task's context
    context_restore(&to->context);

    // This should never return, as context_restore executes iretq
    // If we get here, something went wrong
    __asm__ volatile("sti");
}

// Idle task - runs when no other tasks are ready
void idle_task(void) {
    idle_cpu_has_run = true;
    while (1) {
        /* UEFI poll path: sti;hlt can hang on latent IRQ. Must timer_poll here or
         * sleepers/wait-queue wakes never advance when only idle is runnable. */
        if (timer_is_poll_mode()) {
            timer_poll();
            __asm__ volatile("pause");
        } else {
            __asm__ volatile("sti; hlt" ::: "memory");
        }
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

    TaskStruct* task = task_pool_alloc();
    if (!task) {
        return NULL;
    }

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

    if (priority != PRIORITY_IDLE) {
        if (task_attach_private_as(task) != 0) {
            task_free_stack(task->stack_base);
            task->stack_base = NULL;
            return NULL;
        }
    }

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

                if (task->address_space.pml4_phys != 0 &&
                    task->address_space.pml4_phys != g_kernel_pml4_phys) {
                    vmm_free_pml4(task->address_space.pml4_phys, g_kernel_pml4_phys);
                    task->address_space.pml4_phys = g_kernel_pml4_phys;
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