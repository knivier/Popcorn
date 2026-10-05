#ifndef PHASE2_SELFTEST_H
#define PHASE2_SELFTEST_H

/* Boot-time checks for Phase 2 exit criteria (ioctl device + sleep/IRQ wake).
 * Emits debugcon/serial tags: I (SYS_IOCTL/OPEN/WRITE→dev), B (wait-queue wake),
 * S (sleep wake), 2 (all ok). */
void phase2_selftest(void);

#endif /* PHASE2_SELFTEST_H */
