// src/core/scheduler.c
#include "../includes/scheduler.h"
#include "../includes/timer.h"
#include "../includes/console.h"
#include "../includes/memory.h"
#include "../includes/init.h"
#include "../includes/syscall.h"
#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

#define LINES 25
#define COLUMNS_IN_LINE 80
#define BYTES_FOR_EACH_ELEMENT 2
#define SCREENSIZE BYTES_FOR_EACH_ELEMENT * COLUMNS_IN_LINE * LINES

#define KEYBOARD_DATA_PORT 0x60
#define KEYBOARD_STATUS_PORT 0x64
#define IDT_SIZE 256
#define INTERRUPT_GATE 0x8e
#define KERNEL_CODE_SEGMENT_OFFSET 0x08

#define ENTER_KEY_CODE 0x1C
#define BACKSPACE_KEY_CODE 0x0E
#define UP_ARROW_CODE 0x48
#define DOWN_ARROW_CODE 0x50
#define LEFT_ARROW_CODE 0x4B
#define RIGHT_ARROW_CODE 0x4D
#define PAGE_UP_CODE 0x49
#define PAGE_DOWN_CODE 0x51
#define TAB_KEY_CODE 0x0F

extern unsigned char keyboard_map[128];
extern void keyboard_handler(void);
extern void timer_handler(void);
extern char read_port(unsigned short port);
extern void write_port(unsigned short port, unsigned char data);
extern void int_to_str(int num, char *str);

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
    serial_print("DEBUG: Initializing scheduler\n");

    // Initialize scheduler state
    scheduler.current_task = NULL;
    scheduler.next_pid = 1;
    scheduler.scheduler_active = false;
    scheduler.total_tasks = 0;

    // Initialize ready queues
    for (int i = 0; i < 5; i++) {
        scheduler.ready_queue[i] = NULL;
    }

    serial_print("DEBUG: Creating idle task\n");
    // Create idle task
    TaskStruct* idle = scheduler_create_task(idle_task, NULL, PRIORITY_IDLE);
    if (idle) {
        idle->pid = 0;  // Special PID for idle task
        scheduler.current_task = idle;
        scheduler.current_task->state = TASK_STATE_RUNNING;
        serial_print("DEBUG: Idle task created successfully\n");
    } else {
        serial_print("ERROR: Failed to create idle task\n");
    }

    scheduler.scheduler_active = true;
    serial_print("DEBUG: Scheduler initialization complete\n");
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

    serial_print("DEBUG: Creating new task\n");

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
    serial_print("DEBUG: Allocating task stack\n");
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

    serial_print("DEBUG: Setting up task context\n");
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
    serial_print("DEBUG: Task created successfully, PID: ");
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

    if (strcmp(command, "help") == 0 || strcmp(command, "halp") == 0) {
        console_newline();
        console_println_color("Available Commands:", CONSOLE_HEADER_COLOR);
        console_draw_separator(console_state.cursor_y, CONSOLE_FG_COLOR);

        console_print_color("  hang", CONSOLE_PROMPT_COLOR);
        console_println(" - Hangs the system in a loop");

        console_print_color("  clear", CONSOLE_PROMPT_COLOR);
        console_println(" - Clears the screen");

        console_print_color("  uptime", CONSOLE_PROMPT_COLOR);
        console_println(" - Prints the system uptime");

        console_print_color("  halt", CONSOLE_PROMPT_COLOR);
        console_println(" - Halts the system");

        console_print_color("  write <filename> <content>", CONSOLE_PROMPT_COLOR);
        console_println(" - Writes content to a file");

        console_print_color("  read <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Reads the content of a file");

        console_print_color("  delete <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Deletes a file");

        console_print_color("  rm <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Removes a file (alias for delete)");

        console_print_color("  mkdir <dirname>", CONSOLE_PROMPT_COLOR);
        console_println(" - Creates a new directory");

        console_print_color("  go <dirname>", CONSOLE_PROMPT_COLOR);
        console_println(" - Changes to the specified directory");

        console_print_color("  back", CONSOLE_PROMPT_COLOR);
        console_println(" - Goes back to the previous directory");

        console_print_color("  ls", CONSOLE_PROMPT_COLOR);
        console_println(" - Lists files and directories in current directory");

        console_print_color("  search <filename>", CONSOLE_PROMPT_COLOR);
        console_println(" - Searches for a file and shows its location");

        console_print_color("  cp <filename> <directory>", CONSOLE_PROMPT_COLOR);
        console_println(" - Copies a file to another directory");

        console_print_color("  listsys", CONSOLE_PROMPT_COLOR);
        console_println(" - Lists the entire file system hierarchy");

        console_print_color("  sysinfo", CONSOLE_PROMPT_COLOR);
        console_println(" - Displays detailed system information");

        console_print_color("  mem [option]", CONSOLE_PROMPT_COLOR);
        console_println(" - Memory commands: -map, -use, -stats, -info, -debug");

        console_print_color("  tasks", CONSOLE_PROMPT_COLOR);
        console_println(" - Show current task information");

        console_print_color("  timer", CONSOLE_PROMPT_COLOR);
        console_println(" - Show timer information");
        console_print_color("  syscalls", CONSOLE_PROMPT_COLOR);
        console_println(" - Show system call table");

        console_print_color("  debugtask", CONSOLE_PROMPT_COLOR);
        console_println(" - Create a debug task for testing context switching");

        console_print_color("  killtask", CONSOLE_PROMPT_COLOR);
        console_println(" - Kill the test task to regain control");

        console_println_color("Task Monitor Commands:", CONSOLE_INFO_COLOR);
        console_print_color("  mon -debug", CONSOLE_PROMPT_COLOR);
        console_println(" - Start a debug task");
        console_print_color("  mon -debug [pid]", CONSOLE_PROMPT_COLOR);
        console_println(" - Start debug task with custom PID");
        console_print_color("  mon -list", CONSOLE_PROMPT_COLOR);
        console_println(" - List all running tasks");
        console_print_color("  mon -kill [pid]", CONSOLE_PROMPT_COLOR);
        console_println(" - Kill specific task by PID");
        console_print_color("  mon -ultramon", CONSOLE_PROMPT_COLOR);
        console_println(" - Kill all tasks except idle");

        console_print_color("  cpu [option]", CONSOLE_PROMPT_COLOR);
        console_println(" - CPU commands: -hz, -info");

        console_print_color("  dol [option]", CONSOLE_PROMPT_COLOR);
        console_println(" - Dolphin text editor: -new, -open, -save, -help");

        console_print_color("  stop", CONSOLE_PROMPT_COLOR);
        console_println(" - Shuts down the system");
    } else if (strcmp(command, "hang") == 0) {
        console_print_warning("System hanging...");
        spinner_pop_func(current_loc);
        uptime_module.pop_function(current_loc + 16);
        while (1) {
            console_print_color("Hanging...", CONSOLE_ERROR_COLOR);
        }
    } else if (strcmp(command, "clear") == 0) {
        console_clear();
        console_draw_header("Popcorn Kernel v0.5");
        console_print_success("Screen cleared!");
    } else if (strcmp(command, "uptime") == 0) {
        console_newline();
        char buffer[64];
        int_to_str(get_tick_count(), buffer);
        console_print_color("Uptime: ", CONSOLE_INFO_COLOR);
        console_print_color(buffer, CONSOLE_FG_COLOR);
        console_println(" ticks");

        int ticks = get_tick_count();
        int ticks_per_second = ticks / 150; 
        int_to_str(ticks_per_second, buffer);
        console_print_color("Estimated seconds: ", CONSOLE_INFO_COLOR);
        console_println_color(buffer, CONSOLE_FG_COLOR);
    } else if (strcmp(command, "halt") == 0) {
        console_print_warning("System halted. Press Enter to continue...");
        while (1) {
            unsigned char status = read_port(KEYBOARD_STATUS_PORT);
            if (status & 0x01) {
                char keycode = read_port(KEYBOARD_DATA_PORT);
                if (keycode == ENTER_KEY_CODE) {
                    console_clear();
                    console_draw_header("Popcorn Kernel v0.5");
                    console_print_success("System resumed!");
                    break;
                }
            }
            halt_module.pop_function(current_loc);
        }
    } else if (strcmp(command, "stop") == 0) {
        console_print_warning("Shutting down...");
        write_port(0x64, 0xFE);  

        asm volatile("hlt");
    } else if (strncmp(command, "write ", 6) == 0) {

        if (command[6] == '\0' || command[6] == ' ') {
            console_print_error("Usage: write <filename> <content>");
            return;
        }

        char filename[21] = {0};
        char content[101] = {0};
        int i = 0;
        int j = 0;
        while (command[6 + i] != ' ' && i < 20 && command[6 + i] != '\0' && 6 + i < 128) {
            filename[i] = command[6 + i];
            i++;
        }
        filename[i] = '\0';

        if (i == 0) {
            console_print_error("Filename cannot be empty");
            return;
        }

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
        for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
            if (scheduler.ready_queue[priority]) {
                next_task = scheduler.ready_queue[priority];
                break;
            }
        }
    }

    if (!next_task) {
        // If current task is idle or no other tasks, just return
        if (scheduler.current_task && scheduler.current_task->pid == 0) {
            return;
        }
        next_task = scheduler.current_task;  // Keep running current task
    }
    
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

    if (next_task != scheduler.current_task) {
        TaskStruct* old_task = scheduler.current_task;


        // Update task states
        if (old_task && old_task->state == TASK_STATE_RUNNING) {
            old_task->state = TASK_STATE_READY;
        }

        next_task->state = TASK_STATE_RUNNING;
        next_task->time_remaining = next_task->time_slice;

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

TaskStruct* scheduler_get_current_task(void) {
    return scheduler.current_task;
}

uint32_t scheduler_get_task_count(void) {
    return scheduler.total_tasks;
}

// Print all tasks
void scheduler_print_tasks(void) {
    console_println_color("Task List:", CONSOLE_INFO_COLOR);
    console_println_color("PID | State    | Priority | Runtime", CONSOLE_FG_COLOR);
    console_println_color("----|----------|----------|--------", CONSOLE_FG_COLOR);
    
    if (scheduler.current_task && scheduler.current_task->pid == 0) {
        console_print_color("0   | Running  | Idle     | ", CONSOLE_FG_COLOR);
        char buffer[32];
        int_to_str(scheduler.current_task->total_runtime, buffer);
        console_println_color(buffer, CONSOLE_FG_COLOR);
    }
    
    for (int priority = PRIORITY_REALTIME; priority >= PRIORITY_IDLE; priority--) {
        TaskStruct* task = scheduler.ready_queue[priority];
        while (task) {
            if (task->pid != 0) {  // Skip idle task (already printed)
                char buffer[32];
                int_to_str(task->pid, buffer);
                console_print_color(buffer, CONSOLE_FG_COLOR);
                console_print_color("   | ", CONSOLE_FG_COLOR);
                
                switch (task->state) {
                    case TASK_STATE_READY:
                        console_print_color("Ready    | ", CONSOLE_FG_COLOR);
                        break;
                    case TASK_STATE_RUNNING:
                        console_print_color("Running  | ", CONSOLE_FG_COLOR);
                        break;
                    case TASK_STATE_ZOMBIE:
                        console_print_color("Zombie   | ", CONSOLE_FG_COLOR);
                        break;
                    default:
                        console_print_color("Unknown  | ", CONSOLE_FG_COLOR);
                        break;
                }
                
                // Print priority
                switch (priority) {
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
                
                // Print 
                int_to_str(task->total_runtime, buffer);
                console_println_color(buffer, CONSOLE_FG_COLOR);
            }
            task = task->next;
        }
    }
}

// Set task priority
void scheduler_set_priority(uint32_t pid, TaskPriority priority) {
    // Find and  task priority
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
    task->stack_top = NULL;
    
    // Initialize context to zero
    memset(&task->context, 0, sizeof(CPUContext));
    
    task->vruntime = 0;
    task->time_slice = 10;  // 10 ticks default
    task->time_remaining = task->time_slice;
    
    task->task_function = function;
    task->task_data = data;
    
    task->next = NULL;
    task->prev = NULL;
}

void setup_task_context(TaskStruct* task) {
    if (!task || !task->stack_top) {
        serial_print("ERROR: Invalid task or stack_top in setup_task_context\n");
        return;
    }

    if ((uintptr_t)task->stack_top <= (uintptr_t)task->stack_base) {
        serial_print("ERROR: Stack top <= stack base\n");
        return;
    }

    uintptr_t stack_size = (uintptr_t)task->stack_top - (uintptr_t)task->stack_base;
    if (stack_size < 256) {  // Minimum 256 bytes for safety
        serial_print("ERROR: Stack too small\n");
        return;
    }

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

    task->context.cs = 0x08;  
    task->context.ss = 0x10;  
    task->context.ds = 0x10;
    task->context.es = 0x10;
    task->context.fs = 0x10;
    task->context.gs = 0x10;

    task->context.rflags = 0x202;  // IF=1, bit 1=1 (required)

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

    task->context.fpu_control = 0x37F;  // Default FPU control word
    memset(task->context.fpu_state, 0, sizeof(task->context.fpu_state));

    serial_print("DEBUG: Context setup complete for task\n");
}

void task_switch(TaskStruct* from, TaskStruct* to) {
    if (!to) return;

    __asm__ volatile("cli");

    if (from && from != to) {
        context_save(&from->context);
    }

    scheduler.current_task = to;

    context_restore(&to->context);

    __asm__ volatile("sti");
}

void task_exit(void) {
    TaskStruct* current = scheduler_get_current_task();
    if (current) {
        serial_print("DEBUG: Task exiting\n");
        
        current->state = TASK_STATE_ZOMBIE;
            }
}

void idle_task(void) {
    serial_print("DEBUG: Idle task started\n");

    static int counter = 0;
    while (1) {
        counter++;
            if (counter % 10000000 == 0) {
            serial_print("DEBUG: Idle task running\n");
        }
    }
}

// Test task function to demonstrate context switching
void test_task_function(void) {
    serial_print("DEBUG: Test task started\n");

    static int counter = 0;

    while (1) {
        counter++;

        if (counter % 1000 == 0) {
            serial_print("DEBUG: Test task running: ");
            char digit = '0' + (counter / 1000) % 10;
            serial_putc(digit);
            serial_putc('\n');
        }
        if (counter % 10 == 0) {
            scheduler_yield();
        }

        for (volatile int i = 0; i < 10; i++);
        
    }
}

// Debug task function for mon -debug command
void debug_task_function(void) {
    serial_print("DEBUG: Debug task started\n");

    static int counter = 0;

    while (1) {
        counter++;

        if (counter % 1000 == 0) {
            serial_print("DEBUG: Debug task running: ");
            char digit = '0' + (counter / 1000) % 10;
            serial_putc(digit);
            serial_putc('\n');
        }
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

    serial_print("DEBUG: Creating new task with custom PID\n");

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
    serial_print("DEBUG: Allocating task stack\n");
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

    serial_print("DEBUG: Setting up task context\n");
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
    serial_print("DEBUG: Task created successfully, PID: ");
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