#include "../includes/phase2_selftest.h"
#include "../includes/device.h"
#include "../includes/scheduler.h"
#include "../includes/syscall.h"
#include "../includes/timer.h"
#include "../includes/boot_fb.h"
#include <stddef.h>
#include <stdint.h>

/*
 * Phase 2 exit criteria without relying on a full kmain→task context switch
 * (that path is still fragile while idle carries the shell). We exercise the
 * same APIs: SYS_IOCTL/OPEN/WRITE → fd → device, wait-queue park/wake,
 * sleep-deadline wake.
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
    if (winsize[0] != 80 || winsize[1] != 25) {
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

    TaskStruct* parked = scheduler_create_task(p2_noop_task, NULL, PRIORITY_LOW);
    if (!parked) {
        return;
    }

    WaitQueue wq;
    wq.head = NULL;
    if (!scheduler_park(parked, &wq)) {
        return;
    }
    scheduler_wake_all(&wq);
    if (parked->state != TASK_STATE_READY) {
        return;
    }
    boot_serial_putc('B');

    /* Sleep-deadline wake (same path as scheduler_sleep_ms → wake_expired_sleepers). */
    if (!scheduler_arm_sleep(parked, 0)) {
        return;
    }
    scheduler_service_sleepers();
    if (parked->state != TASK_STATE_READY) {
        return;
    }
    boot_serial_putc('S');
    boot_serial_putc('2');
}
