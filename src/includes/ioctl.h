#ifndef POPCORN_IOCTL_H
#define POPCORN_IOCTL_H

#include <stdint.h>

/*
 * Drive ioctl namespaces (high byte = class, low byte = request).
 * Char/fb already use ad-hoc numbers in backends; new info drives use these.
 */
#define POPCORN_IOC_CLASS_CHAR  0x01u
#define POPCORN_IOC_CLASS_FB    0x02u
#define POPCORN_IOC_CLASS_MEM   0x03u
#define POPCORN_IOC_CLASS_CPU   0x04u
#define POPCORN_IOC_CLASS_CLK   0x05u

#define POPCORN_IOC(class, nr) ((((uint64_t)(class)) << 8) | ((uint64_t)(nr) & 0xFFu))

#define IOC_MEM_STATS   POPCORN_IOC(POPCORN_IOC_CLASS_MEM, 1)
#define IOC_CPU_INFO    POPCORN_IOC(POPCORN_IOC_CLASS_CPU, 1)
#define IOC_CPU_FREQ    POPCORN_IOC(POPCORN_IOC_CLASS_CPU, 2)
#define IOC_CLK_TICKS   POPCORN_IOC(POPCORN_IOC_CLASS_CLK, 1)
#define IOC_CLK_UPTIME  POPCORN_IOC(POPCORN_IOC_CLASS_CLK, 2)
#define IOC_CLK_RTC     POPCORN_IOC(POPCORN_IOC_CLASS_CLK, 3)

#endif /* POPCORN_IOCTL_H */
