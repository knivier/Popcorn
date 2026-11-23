/* src/core/stress.c - preemption stress test (`mon -stress`), roadmap 4.5.
 *
 * Three spinner tasks never yield: only timer preemption can take the CPU from
 * them (so the shell staying responsive is itself part of the test). Each round
 * loads a seed-derived pattern into rbx, r8-r15 and xmm0/xmm1, busy-waits long
 * enough for timer interrupts to land and switch tasks in the middle, then checks
 * every register still holds its pattern: a frame switch that loses or mixes up
 * any GPR or FPU state shows up as an error. A fourth task sleeps and wakes in a
 * loop (sleep/wake under load). */
#include "../includes/stress.h"
#include "../includes/console.h"
#include "../includes/scheduler.h"
#include "../includes/timer.h"
#include "../includes/utils.h"
#include <stdbool.h>
#include <stdint.h>

#define STRESS_SPINNERS 3u
#define STRESS_TASKS (STRESS_SPINNERS + 1u) /* + the sleeper */
#define STRESS_SPIN_LOOPS 20000u

static volatile uint64_t g_rounds[STRESS_TASKS];
static volatile uint64_t g_errors[STRESS_TASKS];
static uint64_t g_seen[STRESS_TASKS];
static uint32_t g_pid[STRESS_TASKS];
static uint64_t g_switches_at_start;
static bool g_running;

/* One round: returns 1 if any register lost its pattern across the spin. */
static uint64_t stress_round(uint64_t s) {
    uint64_t bad;
    uint64_t loops = STRESS_SPIN_LOOPS;
    __asm__ volatile(
        "movq %[s], %%rbx\n\t"
        "leaq 1(%[s]), %%r12\n\t"
        "leaq 2(%[s]), %%r13\n\t"
        "leaq 3(%[s]), %%r14\n\t"
        "leaq 4(%[s]), %%r15\n\t"
        "leaq 5(%[s]), %%r8\n\t"
        "leaq 6(%[s]), %%r9\n\t"
        "leaq 7(%[s]), %%r10\n\t"
        "leaq 8(%[s]), %%r11\n\t"
        "movq %%rbx, %%xmm0\n\t"
        "movq %%r15, %%xmm1\n\t"
        "movl %k[n], %%ecx\n\t"
        "1: pause\n\t"
        "decl %%ecx\n\t"
        "jnz 1b\n\t"
        "xorl %%eax, %%eax\n\t"
        "cmpq %[s], %%rbx\n\t"
        "jne 2f\n\t"
        "leaq 1(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r12\n\t" "jne 2f\n\t"
        "leaq 2(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r13\n\t" "jne 2f\n\t"
        "leaq 3(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r14\n\t" "jne 2f\n\t"
        "leaq 4(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r15\n\t" "jne 2f\n\t"
        "leaq 5(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r8\n\t" "jne 2f\n\t"
        "leaq 6(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r9\n\t" "jne 2f\n\t"
        "leaq 7(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r10\n\t" "jne 2f\n\t"
        "leaq 8(%[s]), %%rdx\n\t" "cmpq %%rdx, %%r11\n\t" "jne 2f\n\t"
        "movq %%xmm0, %%rdx\n\t" "cmpq %[s], %%rdx\n\t" "jne 2f\n\t"
        "movq %%xmm1, %%rdx\n\t" "leaq 4(%[s]), %%rcx\n\t" "cmpq %%rcx, %%rdx\n\t" "jne 2f\n\t"
        "jmp 3f\n\t"
        "2: movl $1, %%eax\n\t"
        "3:\n\t"
        : "=&a"(bad)
        : [s] "r"(s), [n] "r"(loops)
        : "rbx", "rcx", "rdx", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15",
          "xmm0", "xmm1", "cc", "memory");
    return bad;
}

static void stress_spinner(void* arg) {
    uint32_t id = (uint32_t)(uintptr_t)arg;
    for (uint64_t n = 0;; n++) {
        /* Different per task and per round, so a frame restored into the wrong task
         * (or a stale one) cannot pass by accident. */
        uint64_t seed = ((uint64_t)(id + 1u) << 56) ^ (n * 0x9E3779B97F4A7C15ULL);
        g_errors[id] += stress_round(seed);
        g_rounds[id]++;
    }
}

static void stress_sleeper(void* arg) {
    uint32_t id = (uint32_t)(uintptr_t)arg;
    for (;;) {
        uint64_t before = timer_get_ticks();
        scheduler_sleep_ms(20);
        /* Woken too early = broken sleep accounting. */
        if (timer_get_ticks() - before < 1u) {
            g_errors[id]++;
        }
        g_rounds[id]++;
    }
}

static void print_u64(const char* label, uint64_t v) {
    char buf[24];
    console_print_color(label, CONSOLE_FG_COLOR);
    uint64_to_str(v, buf);
    console_print_color(buf, CONSOLE_FG_COLOR);
}

static void stress_report(const char* verdict, bool ok) {
    static const char* names[STRESS_TASKS] = {" A=", " B=", " C=", " sleeper="};
    uint64_t errors = 0;
    console_print_color("stress: ", CONSOLE_INFO_COLOR);
    console_print_color(verdict, ok ? CONSOLE_SUCCESS_COLOR : CONSOLE_ERROR_COLOR);
    for (uint32_t i = 0; i < STRESS_TASKS; i++) {
        print_u64(names[i], g_rounds[i]);
        errors += g_errors[i];
    }
    print_u64(" errors=", errors);
    print_u64(" switches=", scheduler_context_switches() - g_switches_at_start);
    console_newline();
}

void stress_command(const char* args) {
    if (args[0] == '\0') {
        if (!g_running) {
            for (uint32_t i = 0; i < STRESS_TASKS; i++) {
                g_rounds[i] = 0;
                g_errors[i] = 0;
                g_seen[i] = 0;
                g_pid[i] = 0;
            }
            g_switches_at_start = scheduler_context_switches();
            for (uint32_t i = 0; i < STRESS_TASKS; i++) {
                void (*fn)(void*) = i < STRESS_SPINNERS ? stress_spinner : stress_sleeper;
                TaskStruct* t = scheduler_create_task((void (*)(void))fn, (void*)(uintptr_t)i,
                                                      PRIORITY_NORMAL);
                if (!t) {
                    console_print_error("stress: could not create task (pool full?)");
                    stress_command("stop");
                    return;
                }
                g_pid[i] = t->pid;
            }
            g_running = true;
            console_print_success("stress: started 3 spinners (never yield) + 1 sleeper");
            console_println("  run 'mon -stress' again to check, 'mon -stress stop' to end");
            return;
        }
        /* Check: every task advanced since the last check, and no register errors. */
        bool progressed = true;
        uint64_t errors = 0;
        for (uint32_t i = 0; i < STRESS_TASKS; i++) {
            uint64_t now = g_rounds[i];
            if (now <= g_seen[i]) {
                progressed = false;
            }
            g_seen[i] = now;
            errors += g_errors[i];
        }
        if (errors == 0 && progressed) {
            stress_report("OK all 4 progressed", true);
        } else {
            stress_report(errors ? "FAIL register corruption" : "FAIL a task stalled", false);
        }
        return;
    }
    if (strcmp(args, "stop") == 0) {
        for (uint32_t i = 0; i < STRESS_TASKS; i++) {
            if (g_pid[i]) {
                (void)scheduler_kill_task(g_pid[i]);
                g_pid[i] = 0;
            }
        }
        if (g_running) {
            stress_report("stopped", true);
        } else {
            console_println("stress: not running");
        }
        g_running = false;
        return;
    }
    console_print_error("Usage: mon -stress [stop]");
}
