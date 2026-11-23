#ifndef EXCEPTION_H
#define EXCEPTION_H

#include "popframe.h"
#include <stdint.h>

/* Vectors 0x00-0x1F. Returns f to resume (#DB/#BP); fatal exceptions never return. */
PopFrame* exception_handle(PopFrame* f);
const char* exception_name(uint64_t vector);
uint64_t exception_breakpoint_count(void);
uint64_t exception_debug_trap_count(void);

#endif /* EXCEPTION_H */
