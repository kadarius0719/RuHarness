// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdint.h>
#include <stdio.h>

typedef union {
    uint64_t x;
    double f;
} raw_double_t;

void driver(double f) {
    raw_double_t u = {.f = f};
    printf("%llx %a %.4f\n", u.x, f, f);
}