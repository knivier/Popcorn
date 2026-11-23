#include "../includes/phase2_selftest.h"
#include "../includes/console.h"
#include "../includes/device.h"
#include "../includes/scheduler.h"
#include "../includes/syscall.h"
#include "../includes/timer.h"
#include "../includes/boot_fb.h"
#include "../includes/irq.h"
#include <stddef.h>
#include <stdint.h>

/*
 * Phase 2 exit criteria: SYS_IOCTL/OPEN/WRITE → fd → device, wait-queue
 * park/wake, sleep-deadline wake. The helper task is only a subject for those
 * state transitions; it is never run, and it is reclaimed before returning.
 */

static void p2_noop_task(void) {
    for (;;) {
        scheduler_yield();
    }
}

static int ioctl_via_syscall_ok(void) {
    syscall_context_t ctx;
    uint16_t winsize[4] = {0, 0, 0, 0};

    /* stdin/out/err are console after fd_table_init on the idle task. */
    ctx.rdi = 1;
    ctx.rsi = 0x540B; /* TIOCGWINSZ */
    ctx.rdx = (uint64_t)(uintptr_t)winsize;
    if (sys_ioctl(&ctx) != 0) {
        return 0;
    }
    /* Linux struct winsize order: ws_row, ws_col — the live console grid. */
    if (winsize[0] == 0 || winsize[1] == 0 ||
        winsize[0] != (uint16_t)CONSOLE_SCROLL_ROWS ||
        winsize[1] != (uint16_t)console_cols()) {
        return 0;
    }

    ctx.rdi = (uint64_t)(uintptr_t)"null";
    ctx.rsi = 0;
    int64_t fd = sys_open(&ctx);
    if (fd < 3) {
        return 0;
    }
    ctx.rdi = (uint64_t)fd;
    ctx.rsi = (uint64_t)(uintptr_t)"ok";
    ctx.rdx = 2;
    if (sys_write(&ctx) != 2) {
        return 0;
    }
    ctx.rdi = (uint64_t)fd;
    if (sys_close(&ctx) != 0) {
        return 0;
    }
    return 1;
}

void phase2_selftest(void) {
    if (!ioctl_via_syscall_ok()) {
        return;
    }
    boot_serial_putc('I');

    /* One clock: PIT ticks advance (same source as SYS_GETTIME / sleep). */
    uint64_t t0 = timer_get_ticks();
    for (uint32_t n = 0; n < 8000000u && timer_get_ticks() < t0 + 3u; n++) {
        timer_poll();
    }
    if (timer_get_ticks() < t0 + 3u) {
        return;
    }

    /*
     * Preemption is live here (scheduler_end_bootstrap already ran). Keep IRQs off
     * from creation to teardown so a tick never picks the helper while it is READY.
     */
    uint64_t irq_flags = irq_save_disable();
    TaskStruct* parked = scheduler_create_task(p2_noop_task, NULL, PRIORITY_LOW);
    if (!parked) {
        irq_restore_flags(irq_flags);
        return;
    }
    const uint32_t parked_pid = parked->pid;

    WaitQueue wq;
    wq.head = NULL;
    bool ok = scheduler_park(parked, &wq);
    if (ok) {
        scheduler_wake_all(&wq);
        ok = parked->state == TASK_STATE_READY;
    }
    if (ok) {
        boot_serial_putc('B');

        /* Sleep-deadline wake (same path as scheduler_sleep_ms → wake_expired_sleepers). */
        ok = scheduler_arm_sleep(parked, 0);
    }
    if (ok) {
        scheduler_service_sleepers();
        ok = parked->state == TASK_STATE_READY;
    }
    if (ok) {
        boot_serial_putc('S');
        boot_serial_putc('2');
    }

    /* Reclaim the helper (slot, stack, PML4) while `wq` is still in scope; it also
     * drops total_tasks back so `mon -debug` sees only the idle/shell carrier. */
    (void)scheduler_kill_task(parked_pid);
    irq_restore_flags(irq_flags);
}
