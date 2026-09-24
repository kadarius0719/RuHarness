#include "ruharness_guard_internal.h"
void ruharness_probe(volatile unsigned char *p) { *p = 1; }
