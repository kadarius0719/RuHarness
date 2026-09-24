/* RuHarness boundary-check runtime: the interface of the generated call
 * wrapper (docs/ORACLE-HARDENING.md §B.3, §B.6). Harness-owned. */
#ifndef RUHARNESS_GUARD_INTERNAL_H
#define RUHARNESS_GUARD_INTERNAL_H
#include <stddef.h>

/* Call `sym` (index into the unit's interface lines) begins; `frame` is the
 * wrapper's own frame address (everything at or above it is the driver's). */
void ruharness_enter(int sym, void *frame);
/* Data-pointer argument `param` of the current call: returns the pointer the
 * unit is given (the original in measure mode, a guarded shadow otherwise). */
void *ruharness_arg(int param, const void *p, size_t elem);
/* A pointer-typed return value, relocated out of any shadow of this call. */
void *ruharness_ret(void *p);
/* The current call ends: integrity check, relocation, copy-back. */
void ruharness_exit(void);
/* The tracing canary (compiled with sanitizer coverage in the measure build). */
void ruharness_probe(volatile unsigned char *p);

#endif
