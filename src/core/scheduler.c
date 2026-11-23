// src/core/scheduler.c
#include "../includes/scheduler.h"
#include "../includes/timer.h"
#include "../includes/console.h"
#include "../includes/memory.h"
#include "../includes/utils.h"
#include "../includes/device.h"
#include "../includes/irq.h"
#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

/*
 * Kernel threads with one kernel stack each, switched by interrupt frames. Every
 * switched-out task is one saved *PopFrame (popframe.h) on its own kernel stack,
 * pointed to by task->frame. A switch happens only on the way out of an interrupt:
 * isr_dispatch hands the current frame to switch_frames(), which stores it in the
 * outgoing task and returns the incoming task's frame; isr_common then restores
 * that frame's FPU state and registers and iretq's into it.
 *
 *   - Timer preemption: scheduler_tick() (timer IRQ, or timer_poll in PIT poll mode)
 *     only sets g_need_resched; the switch happens at IRQ exit
 *     (scheduler_on_irq_exit), or at the next scheduler_preempt_point() in poll mode.
 *   - Voluntary switches (yield/block/sleep/exit) execute `int 0x81`
 *     (scheduler_on_yield), so they take exactly the same path.
 *   - A new task starts from a frame built by setup_task_context(): its first
 *     "resume" iretq's into task_entry_trampoline.
 *
 * Locking: single CPU. The ready queues, wait queues and task pool are changed only
 * with interrupts disabled (irq_save_disable/irq_restore_flags). Each frame carries
 * its own RFLAGS, so a task resumes with the interrupt flag it was switched out with.
 *
 * Invariant: a task is linked on ready_queue[priority] iff its state is READY or
 * RUNNING (the running task stays queued so round-robin can step past it). BLOCKED
 * tasks are linked on exactly one WaitQueue (task->wait_queue); SLEEPING and ZOMBIE
 * tasks are on no list.
 */

// Global scheduler state
SchedulerState scheduler = {0};

/* Boot page-table root (asm identity map); all tasks use this until given another PML4. */
static uint64_t g_kernel_pml4_phys;

/* current_task is the idle TCB (pid 0) while the CPU still runs kmain on the boot stack.
 * Until scheduler_end_bootstrap() no switch is allowed; after it, the idle TCB carries
 * kmain/shell and its saved stack pointer is written by the first switch away from it. */
static bool idle_cpu_has_run;

/* True while scheduler.current_task is synthetic idle but CPU still executes kmain boot stack. */
static bool bootstrap_on_kmain_stack(void) {
    return scheduler.current_task &&
           scheduler.current_task->pid == 0 &&
           !idle_cpu_has_run;
}

/* Set by scheduler_tick (and wakeups that outrank the current task); consumed at
 * IRQ exit or at a poll-mode preempt point. */
static volatile bool g_need_resched;
static uint64_t g_context_switches;

/* Preempt after this many ticks of the same task when others are runnable. */
#define SCHED_QUANTUM_TICKS 5u

#define TASK_POOL_CAP 32u
#define TASK_STACK_SIZE (16 * 1024)  // 16KB per task stack

static TaskStruct g_task_pool[TASK_POOL_CAP];
static bool g_task_used[TASK_POOL_CAP];

/* Static stacks: no dependency on the heap during early boot. */
static char g_task_stacks[TASK_POOL_CAP][TASK_STACK_SIZE] __attribute__((aligned(16)));
static bool g_stack_used[TASK_POOL_CAP];

extern void boot_serial_putc(char c);

static void serial_print(const char* str) {
    while (str && *str) {
        boot_serial_putc(*str++);
    }
}

void scheduler_end_bootstrap(void) {
    /*
     * Entering the interactive shell: allow ticks/yields. The idle TCB becomes the
     * carrier for kmain's context on the first switch away from it.
     */
    idle_cpu_has_run = true;
}

static bool prio_valid(int p) {
    return p >= PRIORITY_IDLE && p <= PRIORITY_REALTIME;
}

static bool task_is_runnable(const TaskStruct* t) {
    return t && (t->state == TASK_STATE_READY || t->state == TASK_STATE_RUNNING);
}

/* ---- Ready queues (IRQs off) ---- */

static bool ready_is_queued(const TaskStruct* task) {
    int p = (int)task->priority;
    return task->prev != NULL || (prio_valid(p) && scheduler.ready_queue[p] == task);
}

static void ready_remove(TaskStruct* task) {
    if (!task) {
        return;
    }
    int p = (int)task->priority;
    if (!prio_valid(p) || !ready_is_queued(task)) {
        task->next = NULL;
        task->prev = NULL;
        return;
    }
    if (task->prev) {
        task->prev->next = task->next;
    } else {
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
    if (!prio_valid(p) || ready_is_queued(task)) {
        return;  // never link a task twice
    }
    task->next = scheduler.ready_queue[p];
    if (scheduler.ready_queue[p]) {
        scheduler.ready_queue[p]->prev = task;
    }
    scheduler.ready_queue[p] = task;
    task->prev = NULL;
}

/* ---- Wait queues (IRQs off) ---- */

static void wait_enqueue(WaitQueue* wq, TaskStruct* t) {
    t->wait_next = wq->head;
    wq->head = t;
    t->wait_queue = wq;
}

static void wait_unlink(TaskStruct* t) {
    WaitQueue* wq = t->wait_queue;
    if (wq) {
        TaskStruct** pp = &wq->head;
        while (*pp) {
            if (*pp == t) {
                *pp = t->wait_next;
                break;
            }
            pp = &(*pp)->wait_next;
        }
    }
    t->wait_next = NULL;
    t->wait_queue = NULL;
}

static void wake_expired_sleepers(void) {
    uint64_t now = timer_get_ticks();
    for (uint32_t i = 0; i < TASK_POOL_CAP; i++) {
        TaskStruct* t = &g_task_pool[i];
        if (g_task_used[i] && t->state == TASK_STATE_SLEEPING && now >= t->sleep_until_tick) {
            t->state = TASK_STATE_READY;
            ready_add(t);
        }
    }
}

/* ---- Task pool and stacks ---- */

static TaskStruct* task_pool_alloc(void) {
    for (uint32_t i = 0; i < TASK_POOL_CAP; i++) {
        if (!g_task_used[i]) {
            g_task_used[i] = true;
            memset(&g_task_pool[i], 0, sizeof(g_task_pool[i]));
            return &g_task_pool[i];
        }
    }
    console_println_color("Task pool exhausted", CONSOLE_ERROR_COLOR);
    serial_print("ERROR: Task pool exhausted\n");
    return NULL;
}

static void task_pool_free(TaskStruct* task) {
    if (task < &g_task_pool[0] || task >= &g_task_pool[TASK_POOL_CAP]) {
        return;
    }
    task->state = TASK_STATE_ZOMBIE;
    task->next = NULL;
    task->prev = NULL;
    task->wait_next = NULL;
    task->wait_queue = NULL;
    g_task_used[task - g_task_pool] = false;
}

void* task_allocate_stack(uint64_t size) {
    if (size > TASK_STACK_SIZE) {
        return NULL;
    }
    uint64_t flags = irq_save_disable();
    void* stack = NULL;
    for (uint32_t i = 0; i < TASK_POOL_CAP; i++) {
        if (!g_stack_used[i]) {
            g_stack_used[i] = true;
            stack = g_task_stacks[i];
            break;
        }
    }
    irq_restore_flags(flags);
    if (stack) {
        memset(stack, 0, TASK_STACK_SIZE);
    }
    return stack;
}

void task_free_stack(void* stack) {
    uintptr_t base = (uintptr_t)g_task_stacks;
    uintptr_t p = (uintptr_t)stack;
    if (!stack || p < base || p >= base + sizeof(g_task_stacks) ||
        ((p - base) % TASK_STACK_SIZE) != 0) {
        return;
    }
    uint64_t flags = irq_save_disable();
    g_stack_used[(p - base) / TASK_STACK_SIZE] = false;
    irq_restore_flags(flags);
}

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

static void task_release_address_space(TaskStruct* task) {
    if (task->address_space.pml4_phys != 0 &&
        task->address_space.pml4_phys != g_kernel_pml4_phys) {
        vmm_free_pml4(task->address_space.pml4_phys, g_kernel_pml4_phys);
    }
    task->address_space.pml4_phys = g_kernel_pml4_phys;
}

/* Unlink a task from whatever list it is on and mark it ZOMBIE (IRQs off). */
static void task_make_zombie(TaskStruct* t) {
    if (!t || t->state == TASK_STATE_ZOMBIE) {
        return;
    }
    ready_remove(t);
    wait_unlink(t);
    t->state = TASK_STATE_ZOMBIE;
    t->sleep_until_tick = 0;
    if (scheduler.total_tasks > 0) {
        scheduler.total_tasks--;
    }
}

/* Return a ZOMBIE's fds, PML4, stack and pool slot. Never for the running task, whose
 * stack and CR3 are still in use (IRQs off). */
static void task_reclaim(TaskStruct* t) {
    if (!t || t == scheduler.current_task || t->state != TASK_STATE_ZOMBIE) {
        return;
    }
    for (int fd = 0; fd < FD_MAX; fd++) {
        (void)fd_close(t->fds, fd);
    }
    task_release_address_space(t);
    task_free_stack(t->stack_base);
    t->stack_base = NULL;
    t->stack_top = NULL;
    task_pool_free(t);
}

/* Reclaim tasks that exited themselves (task context only: vmm_free_pml4 is not IRQ-safe). */
static void reap_zombies(void) {
    for (uint32_t i = 0; i < TASK_POOL_CAP; i++) {
        if (g_task_used[i] && g_task_pool[i].state == TASK_STATE_ZOMBIE) {
            task_reclaim(&g_task_pool[i]);
        }
    }
}

static TaskStruct* find_live_task(uint32_t pid) {
    for (uint32_t i = 0; i < TASK_POOL_CAP; i++) {
        TaskStruct* t = &g_task_pool[i];
        if (g_task_used[i] && t->state != TASK_STATE_ZOMBIE && t->pid == pid) {
            return t;
        }
    }
    return NULL;
}

/* ---- Picking and switching ---- */

/*
 * Priority first, round-robin within a level:
 *   1. the head of the highest non-empty queue above the current task's priority
 *      (all levels when the current task is blocked/sleeping/exiting);
 *   2. the next task at the current priority;
 *   3. the head of the highest non-empty lower queue (so a lone task still lets
 *      lower levels, e.g. the idle/shell carrier, run when it yields);
 *   4. the current task itself if it is still runnable, else NULL.
 */
/* Anti-starvation: with strict priority, two runnable tasks at one level only
 * ever round-robin with each other, so every lower level (including the shell,
 * which rides the PRIORITY_IDLE TCB) never ran again ("mon -debug" twice hung
 * the shell). Every STARVE_PICKS-th pick that passes over a non-empty lower
 * queue runs that queue's head instead. Counts picks, not ticks: in UEFI poll
 * mode ticks only advance when someone calls timer_poll(). */
#define STARVE_PICKS 4u
static uint32_t g_lower_passed_over;

/* Round-robin cursor per level: who runs next when the CPU comes back to a level
 * from below. Without it the queue head got an extra slice every time a lower level
 * (e.g. the shell's anti-starvation turn) ran, so one task did twice the work. */
static TaskStruct* g_rr_next[PRIORITY_REALTIME + 1];

static TaskStruct* rr_pick(int p) {
    TaskStruct* t = g_rr_next[p];
    if (t && (int)t->priority == p && task_is_runnable(t) && ready_is_queued(t)) {
        return t;
    }
    return scheduler.ready_queue[p];
}

static TaskStruct* pick_next(TaskStruct* cur) {
    const bool cur_runnable = task_is_runnable(cur) && prio_valid((int)cur->priority);
    const int cur_prio = cur_runnable ? (int)cur->priority : PRIORITY_IDLE - 1;

    for (int p = PRIORITY_REALTIME; p > cur_prio; p--) {
        if (scheduler.ready_queue[p]) {
            return rr_pick(p);
        }
    }
    if (!cur_runnable) {
        return NULL;
    }

    TaskStruct* lower = NULL;
    for (int p = cur_prio - 1; p >= PRIORITY_IDLE; p--) {
        if (scheduler.ready_queue[p]) {
            lower = scheduler.ready_queue[p];
            break;
        }
    }

    TaskStruct* cand = cur->next ? cur->next : scheduler.ready_queue[cur_prio];
    if (cand && cand != cur) {
        if (lower && ++g_lower_passed_over >= STARVE_PICKS) {
            g_lower_passed_over = 0;
            return lower;
        }
        return cand;
    }
    if (lower) {
        g_lower_passed_over = 0;
        return lower;
    }
    return cur;
}

/* Interrupt context, IRQs off. Store f as the current task's saved frame, pick the
 * next task and return the frame to resume (f itself when nobody else should run). */
static PopFrame* switch_frames(PopFrame* f) {
    TaskStruct* cur = scheduler.current_task;
    g_need_resched = false;
    if (!scheduler.scheduler_active || !cur || bootstrap_on_kmain_stack()) {
        return f;
    }

    TaskStruct* next = pick_next(cur);
    if (!next || next == cur) {
        if (cur->state == TASK_STATE_READY) {
            cur->state = TASK_STATE_RUNNING;
        }
        return f;
    }
    if (!next->frame || ((uintptr_t)next->frame & 15u) != 0) {
        serial_print("ERROR: Invalid saved frame for switching\n");
        return f;
    }

    cur->frame = f;
    if (task_is_runnable(cur) && prio_valid((int)cur->priority)) {
        int p = (int)cur->priority;
        g_rr_next[p] = cur->next ? cur->next : scheduler.ready_queue[p];
    }

    /* Charge the outgoing task up to now; the incoming one starts a fresh run. */
    uint64_t now = timer_get_ticks();
    cur->total_runtime += now - cur->last_run_time;
    cur->last_run_time = now;
    next->last_run_time = now;

    if (cur->state == TASK_STATE_RUNNING) {
        cur->state = TASK_STATE_READY;
    }
    next->state = TASK_STATE_RUNNING;
    next->time_remaining = next->time_slice;

    /* Switch address space while still on the outgoing stack (kernel half is shared). */
    if (next->address_space.pml4_phys != 0) {
        uint64_t want = next->address_space.pml4_phys & VMM_PTE_ADDR_MASK;
        uint64_t have = vmm_get_cr3() & VMM_PTE_ADDR_MASK;
        if (want != have) {
            vmm_load_cr3(want);
        }
    }

    scheduler.current_task = next;
    g_context_switches++;
    return next->frame;
}

/* isr_dispatch, vector 0x81. */
PopFrame* scheduler_on_yield(PopFrame* f) {
    return switch_frames(f);
}

/* isr_dispatch, after an IRQ handler ran. Never inside a USB/xHCI quiet section. */
PopFrame* scheduler_on_irq_exit(PopFrame* f) {
    if (g_need_resched && !irq_quiet_active()) {
        return switch_frames(f);
    }
    return f;
}

/* Voluntary switch: the same frame path as preemption. Returns when the current task
 * runs again (immediately if nothing else was picked), with its own RFLAGS back. */
static void schedule_locked(void) {
    if (!scheduler.scheduler_active || !scheduler.current_task || bootstrap_on_kmain_stack()) {
        return;
    }
    __asm__ volatile("int $0x81" ::: "memory");
}

/* PIT poll mode has no timer IRQ to switch on: code that polls the timer calls this
 * where a switch is safe (task context, no locks held). */
void scheduler_preempt_point(void) {
    if (g_need_resched && !irq_quiet_active() && !bootstrap_on_kmain_stack()) {
        schedule_locked();
    }
}

uint64_t scheduler_context_switches(void) {
    return g_context_switches;
}

/* Wait in place for an interrupt / poll tick (IRQs off on entry and exit). */
static void idle_wait_once(void) {
    if (timer_is_poll_mode()) {
        /* UEFI poll path: sti;hlt can hang on a latent IRQ; ticks only advance here. */
        timer_poll();
        __asm__ volatile("pause");
    } else {
        /* sti's one-instruction shadow: an IRQ cannot slip in before hlt. */
        __asm__ volatile("sti; hlt; cli" ::: "memory");
    }
}

/* Current task t just became BLOCKED/SLEEPING (IRQs off). Run others until t is woken;
 * when nothing else is runnable, idle here so the wake (IRQ or poll tick) can happen. */
static void wait_until_runnable(TaskStruct* t) {
    while (!task_is_runnable(t)) {
        schedule_locked();
        if (task_is_runnable(t)) {
            break;
        }
        idle_wait_once();
    }
    t->state = TASK_STATE_RUNNING;
}

void scheduler_task_entry(void (*fn)(void), void* data) {
    /* Arrived by iretq from the initial frame with IF=0. New tasks run with interrupts
     * on, except on the PIT poll-mode fallback path. */
    if (!timer_is_poll_mode()) {
        __asm__ volatile("sti" ::: "memory");
    }
    if (fn) {
        /* Task functions may take their data argument (RDI) or ignore it. */
        ((void (*)(void*))fn)(data);
    }
    scheduler_exit_current();
}

void scheduler_exit_current(void) {
    (void)irq_save_disable();
    TaskStruct* t = scheduler.current_task;
    /* The idle TCB carries kmain/shell and cannot exit; it just stops returning. */
    if (t && t->pid != 0) {
        task_make_zombie(t);
        schedule_locked();
    }
    /* Nothing else was runnable yet: idle on this stack until a switch takes us away.
     * The stack is not reclaimed while this task is still current. */
    for (;;) {
        idle_wait_once();
        schedule_locked();
    }
}

// Initialize the scheduler
static TaskStruct* task_create_internal(void (*function)(void), void* data, TaskPriority priority,
                                        bool fixed_pid, uint32_t pid);

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

    // Create idle task (PID 0)
    idle_cpu_has_run = false;
    TaskStruct* idle = task_create_internal(idle_task, NULL, PRIORITY_IDLE, true, 0);
    if (idle) {
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

// Scheduler tick handler (timer IRQ after EOI, or timer_poll in poll mode)
void scheduler_tick(void) {
    // Disable scheduler tick during early boot to avoid crashes
    static bool first_tick = true;
    static int tick_counter = 0;
    if (first_tick) {
        first_tick = false;
        return;  // Skip first tick to avoid early crashes
    }

    if (!scheduler.scheduler_active || !scheduler.current_task) {
        return;
    }

    uint64_t flags = irq_save_disable();
    wake_expired_sleepers();

    /* Until scheduler_end_bootstrap(), skip preempt — still on kmain boot stack. */
    if (!bootstrap_on_kmain_stack()) {
        TaskStruct* cur = scheduler.current_task;

        // Update current task runtime
        uint64_t current_time = timer_get_ticks();
        cur->total_runtime += current_time - cur->last_run_time;
        cur->last_run_time = current_time;

        // Decrease time slice
        if (cur->time_remaining > 0) {
            cur->time_remaining--;
        }

        /* Ask for a switch every quantum, or when the slice runs out. The switch itself
         * happens at IRQ exit (scheduler_on_irq_exit) / the next poll-mode preempt point. */
        tick_counter++;
        if (tick_counter >= (int)SCHED_QUANTUM_TICKS) {
            tick_counter = 0;
            g_need_resched = true;
        } else if (cur->time_remaining == 0 && scheduler.total_tasks > 1) {
            g_need_resched = true;
        }
    }
    irq_restore_flags(flags);
}

// Yield CPU to another task
void scheduler_yield(void) {
    if (scheduler.scheduler_active && !bootstrap_on_kmain_stack()) {
        scheduler_schedule();
    }
}

void scheduler_block(WaitQueue* wq) {
    TaskStruct* t = scheduler.current_task;
    if (!t || !wq || bootstrap_on_kmain_stack()) {
        return;
    }
    uint64_t flags = irq_save_disable();
    /* Enqueue and switch in one IRQ-off section: no lost wakeup in between. */
    ready_remove(t);
    t->state = TASK_STATE_BLOCKED;
    wait_enqueue(wq, t);
    wait_until_runnable(t);
    irq_restore_flags(flags);
}

void scheduler_wake_one(WaitQueue* wq) {
    if (!wq) {
        return;
    }
    uint64_t flags = irq_save_disable();
    TaskStruct* t = wq->head;
    if (t) {
        wq->head = t->wait_next;
        t->wait_next = NULL;
        t->wait_queue = NULL;
        if (t->state == TASK_STATE_BLOCKED) {
            t->state = TASK_STATE_READY;
            ready_add(t);
        }
    }
    irq_restore_flags(flags);
}

void scheduler_wake_all(WaitQueue* wq) {
    if (!wq) {
        return;
    }
    uint64_t flags = irq_save_disable();
    while (wq->head) {
        scheduler_wake_one(wq);
    }
    irq_restore_flags(flags);
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
    uint64_t ticks = timer_ms_to_ticks(ms);
    if (ticks == 0) {
        ticks = 1;
    }
    uint64_t flags = irq_save_disable();
    ready_remove(t);
    t->sleep_until_tick = timer_get_ticks() + ticks;
    t->state = TASK_STATE_SLEEPING;
    wait_until_runnable(t);
    irq_restore_flags(flags);
}

bool scheduler_park(TaskStruct* t, WaitQueue* wq) {
    if (!t || !wq || t == scheduler.current_task) {
        return false;  // the running task must use scheduler_block()
    }
    uint64_t flags = irq_save_disable();
    if (t->state == TASK_STATE_ZOMBIE) {
        irq_restore_flags(flags);
        return false;
    }
    ready_remove(t);
    wait_unlink(t);
    t->state = TASK_STATE_BLOCKED;
    wait_enqueue(wq, t);
    irq_restore_flags(flags);
    return true;
}

bool scheduler_arm_sleep(TaskStruct* t, uint64_t sleep_until_tick) {
    if (!t || t == scheduler.current_task) {
        return false;  // the running task must use scheduler_sleep_ms()
    }
    uint64_t flags = irq_save_disable();
    if (t->state == TASK_STATE_ZOMBIE) {
        irq_restore_flags(flags);
        return false;
    }
    ready_remove(t);
    wait_unlink(t);
    t->sleep_until_tick = sleep_until_tick;
    t->state = TASK_STATE_SLEEPING;
    irq_restore_flags(flags);
    return true;
}

void scheduler_service_sleepers(void) {
    uint64_t flags = irq_save_disable();
    wake_expired_sleepers();
    irq_restore_flags(flags);
}

static bool pid_in_use(uint32_t pid) {
    return find_live_task(pid) != NULL;
}

/* Shared by scheduler_create_task / _with_pid / the idle task. */
static TaskStruct* task_create_internal(void (*function)(void), void* data, TaskPriority priority,
                                        bool fixed_pid, uint32_t pid) {
    if (!function) {
        serial_print("ERROR: No function provided for task creation\n");
        return NULL;
    }
    if (!prio_valid((int)priority)) {
        serial_print("ERROR: Invalid task priority\n");
        return NULL;
    }

    uint64_t flags = irq_save_disable();
    reap_zombies();

    if (fixed_pid && pid_in_use(pid)) {
        irq_restore_flags(flags);
        serial_print("ERROR: PID already in use\n");
        return NULL;
    }

    TaskStruct* task = task_pool_alloc();
    if (!task) {
        irq_restore_flags(flags);
        return NULL;
    }

    // Initialize task
    task_init(task, function, data, priority);
    if (fixed_pid) {
        task->pid = pid;
    } else {
        while (scheduler.next_pid == 0 || pid_in_use(scheduler.next_pid)) {
            scheduler.next_pid++;
        }
        task->pid = scheduler.next_pid++;
    }
    task->start_time = timer_get_ticks();
    task->last_run_time = task->start_time;

    // Allocate and set up stack
    task->stack_size = TASK_STACK_SIZE;
    task->stack_base = task_allocate_stack(task->stack_size);
    if (!task->stack_base) {
        task_pool_free(task);
        irq_restore_flags(flags);
        console_println_color("Failed to allocate task stack", CONSOLE_ERROR_COLOR);
        serial_print("ERROR: Failed to allocate task stack\n");
        return NULL;
    }

    // Stack grows downward; keep the top 16-byte aligned (x86-64 ABI)
    uintptr_t stack_top_addr = (uintptr_t)task->stack_base + task->stack_size;
    stack_top_addr = stack_top_addr & ~((uintptr_t)15);
    task->stack_top = (void*)stack_top_addr;

    // Set up initial context for the task
    setup_task_context(task);

    /* Idle keeps the boot master CR3; everyone else gets a private PML4. */
    if (task->frame == NULL ||
        (priority != PRIORITY_IDLE && task_attach_private_as(task) != 0)) {
        task_free_stack(task->stack_base);
        task->stack_base = NULL;
        task_pool_free(task);
        irq_restore_flags(flags);
        return NULL;
    }

    // Add to ready queue
    task->state = TASK_STATE_READY;
    ready_add(task);
    scheduler.total_tasks++;
    irq_restore_flags(flags);
    return task;
}

// Create a new task
TaskStruct* scheduler_create_task(void (*function)(void), void* data, TaskPriority priority) {
    return task_create_internal(function, data, priority, false, 0);
}

// Create a task with a specific PID (0 is reserved for the idle/shell carrier)
TaskStruct* scheduler_create_task_with_pid(void (*function)(void), void* data, TaskPriority priority, uint32_t custom_pid) {
    if (custom_pid == 0) {
        serial_print("ERROR: PID 0 is reserved for the idle task\n");
        return NULL;
    }
    return task_create_internal(function, data, priority, true, custom_pid);
}

bool scheduler_kill_task(uint32_t pid) {
    if (pid == 0) {
        return false;  // idle TCB carries kmain/shell
    }
    uint64_t flags = irq_save_disable();
    reap_zombies();
    TaskStruct* task = find_live_task(pid);
    if (!task) {
        irq_restore_flags(flags);
        return false;
    }
    if (task == scheduler.current_task) {
        scheduler_exit_current();
    }
    /* Any state: READY/RUNNING leave the ready queue, BLOCKED leaves its wait queue,
     * SLEEPING is simply never woken. Not running, so reclaim right away. */
    task_make_zombie(task);
    task_reclaim(task);
    irq_restore_flags(flags);
    return true;
}

// Destroy a task
void scheduler_destroy_task(uint32_t pid) {
    (void)scheduler_kill_task(pid);
}

// Main scheduling function
void scheduler_schedule(void) {
    uint64_t flags = irq_save_disable();
    schedule_locked();
    irq_restore_flags(flags);
}

/* For Rust's TaskLock (sync.rs): who holds a lock, and whether that task still
 * exists (a killed holder must not wedge the lock forever). */
uint32_t scheduler_current_pid(void) {
    TaskStruct* t = scheduler.current_task;
    return t ? t->pid : 0u;
}

int scheduler_pid_alive(uint32_t pid) {
    uint64_t flags = irq_save_disable();
    int alive = find_live_task(pid) != NULL;
    irq_restore_flags(flags);
    return alive;
}

// Get current running task
TaskStruct* scheduler_get_current_task(void) {
    return scheduler.current_task;
}

// Get total number of tasks
uint32_t scheduler_get_task_count(void) {
    return scheduler.total_tasks;
}

// Print all live tasks (any state)
void scheduler_print_tasks(void) {
    console_println_color("PID | State    | Priority | Runtime", CONSOLE_FG_COLOR);
    console_println_color("----|----------|----------|--------", CONSOLE_FG_COLOR);

    for (uint32_t i = 0; i < TASK_POOL_CAP; i++) {
        TaskStruct* task = &g_task_pool[i];
        if (!g_task_used[i] || task->state == TASK_STATE_ZOMBIE) {
            continue;
        }
        char buffer[32];
        int_to_str((int)task->pid, buffer);
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
        int_to_str((int)task->total_runtime, buffer);
        console_println_color(buffer, CONSOLE_FG_COLOR);
    }
}

// Set task priority (re-queues a runnable task on its new level)
void scheduler_set_priority(uint32_t pid, TaskPriority priority) {
    if (!prio_valid((int)priority)) {
        return;
    }
    uint64_t flags = irq_save_disable();
    TaskStruct* task = find_live_task(pid);
    if (task) {
        bool queued = task_is_runnable(task);
        if (queued) {
            ready_remove(task);
        }
        task->priority = priority;
        if (queued) {
            ready_add(task);
        }
    }
    irq_restore_flags(flags);
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
    task->frame = NULL;

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
    task->wait_queue = NULL;
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
    uintptr_t top = (uintptr_t)task->stack_top & ~((uintptr_t)15);
    if (top - (uintptr_t)task->stack_base < 256) {  // Minimum 256 bytes for safety
        serial_print("ERROR: Stack too small\n");
        return;
    }

    /*
     * Initial *PopFrame, resumed like any preempted task: isr_common restores the
     * FXSAVE area below it and the GPRs, then iretq's into task_entry_trampoline with
     * IF=0, r12 = function, r13 = data and RSP = top - 8 (as if called). The trampoline
     * calls scheduler_task_entry(), which runs the function and exits the task if it
     * ever returns. Layout below top: 16 spare bytes, the PopFrame, the FXSAVE area.
     */
    if (top - (uintptr_t)task->stack_base < 16u + sizeof(PopFrame) + POPFRAME_FPU_BYTES + 512u) {
        serial_print("ERROR: Stack too small for the initial frame\n");
        return;
    }
    PopFrame* f = (PopFrame*)(top - 16u - sizeof(PopFrame));
    uint8_t* fpu = POPFRAME_FPU(f);
    memset(fpu, 0, POPFRAME_FPU_BYTES + sizeof(PopFrame));
    *(uint16_t*)(void*)(fpu + 0) = 0x037F;   /* FCW: x87 power-on default */
    *(uint32_t*)(void*)(fpu + 24) = 0x1F80;  /* MXCSR: all SSE exceptions masked */

    uint16_t cs, ss;
    __asm__ volatile("mov %%cs, %0" : "=r"(cs));
    __asm__ volatile("mov %%ss, %0" : "=r"(ss));
    f->rip = (uint64_t)(uintptr_t)task_entry_trampoline;
    f->cs = cs;
    f->rflags = 0x2;  /* IF=0: scheduler_task_entry turns interrupts on */
    f->rsp = (uint64_t)(top - 8u);
    f->ss = ss;
    f->r12 = (uint64_t)(uintptr_t)task->task_function;
    f->r13 = (uint64_t)(uintptr_t)task->task_data;
    task->frame = f;

    memset(&task->context, 0, sizeof(task->context));
    task->context.rsp = f->rsp;

    /* Informational only (not loaded by the switch). */
    task->context.rip = (uint64_t)(uintptr_t)task->task_function;
    task->context.rdi = (uint64_t)(uintptr_t)task->task_data;
    task->context.rflags = 0x202;  // entry runs with IF=1 except in PIT poll mode
    task->context.cs = cs;
    task->context.ss = ss;
    task->context.fpu_control = 0x37F;
}

// Idle task - runs only if the idle TCB is ever entered fresh (it normally carries kmain)
void idle_task(void) {
    idle_cpu_has_run = true;
    while (1) {
        /* UEFI poll path: sti;hlt can hang on latent IRQ. Must timer_poll here or
         * sleepers/wait-queue wakes never advance when only idle is runnable. */
        if (timer_is_poll_mode()) {
            timer_poll();
            scheduler_preempt_point();
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

// Kill all tasks except the idle task (PID 0), in any state
void scheduler_kill_all_except_idle(void) {
    uint64_t flags = irq_save_disable();
    TaskStruct* self = scheduler.current_task;
    bool kill_self = false;

    for (uint32_t i = 0; i < TASK_POOL_CAP; i++) {
        TaskStruct* task = &g_task_pool[i];
        if (!g_task_used[i] || task->pid == 0) {
            continue;
        }
        if (task == self) {
            kill_self = task->state != TASK_STATE_ZOMBIE;
            continue;
        }
        task_make_zombie(task);
        task_reclaim(task);
    }

    if (kill_self) {
        scheduler_exit_current();
    }
    irq_restore_flags(flags);
}
