#include "proj.h"

/* A smaller decoder for small builds. */
int decode(const char *s) { return s[0] < 'a' ? 0 : 1; }
