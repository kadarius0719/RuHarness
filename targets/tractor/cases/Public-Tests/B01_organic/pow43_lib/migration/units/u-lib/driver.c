#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>
#include <limits.h>
#include <float.h>
#include <math.h>
#include <stdbool.h>
#include <ctype.h>
#include <errno.h>

/* pow43's table lookup is only defined for x in [-16, 8191]; outside that
   range the table index computed inside the unit runs off the end of the
   static array, so the driver never probes beyond this implied
   precondition. */
#define POW43_MIN (-16)
#define POW43_MAX 8191

static uint64_t g_rng_state = 0x243F6A8885A308D3ULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static int rand_int_range(int lo, int hi) {
    uint64_t r = xorshift64();
    uint64_t span = (uint64_t)((int64_t)hi - (int64_t)lo) + 1ULL;
    uint64_t v = r % span;
    return (int)((int64_t)lo + (int64_t)v);
}

static int g_case = 0;

static void run_case(int x) {
    float r = pow43(x);
    printf("case %d x=%d ret=%a\n", g_case, x, r);
    g_case++;
}

int main(void) {
    static const int fixed_x[] = {
        -16, -15, -8, -4, -1, 0, 1, 2, 4, 8, 16, 32, 64,
        127, 128, 129, 130, 200, 500, 1000,
        1023, 1024, 1025, 2000, 4000, 8000, 8100, 8191
    };
    size_t n = sizeof(fixed_x) / sizeof(fixed_x[0]);
    size_t i;

    for (i = 0; i < n; i++) {
        run_case(fixed_x[i]);
    }

    for (i = 0; i < 400; i++) {
        int x = rand_int_range(POW43_MIN, POW43_MAX);
        run_case(x);
    }

    return 0;
}
