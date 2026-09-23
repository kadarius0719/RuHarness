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

static uint32_t xr_state;
static int g_case = 0;

static uint32_t xr32(void) {
    uint32_t x = xr_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    xr_state = x;
    return x;
}

static void run(int v1, int v2) {
    if (v1 == INT_MIN && v2 == -1) {
        /* The unit's own INT_MIN branch overflows internally for this
           exact pair (the true quotient, 2147483648, has no int
           representation); avoid feeding it this precondition-violating
           input and use a nearby safe divisor instead. */
        v2 = 1;
    }
    int r = div_euclid(v1, v2);
    g_case++;
    printf("case %d v1=%d v2=%d ret=%d\n", g_case, v1, v2, r);
}

int main(void) {
    xr_state = 0x13572468u;

    static const int VALS[] = {
        INT_MIN, INT_MIN + 1, -1000000, -17, -2, -1, 0, 1, 2, 17, 1000000, INT_MAX
    };
    size_t n = sizeof(VALS) / sizeof(VALS[0]);

    /* Full cross product: covers division by zero, every sign combination,
       and the INT_MIN special cases on both operands. */
    for (size_t i = 0; i < n; i++) {
        for (size_t j = 0; j < n; j++) {
            run(VALS[i], VALS[j]);
        }
    }

    /* A spread of everyday values not already covered above. */
    static const int MORE[] = {
        3, -3, 7, -7, 10, -10, 100, -100, 12345, -12345, 999983, -999983
    };
    size_t m = sizeof(MORE) / sizeof(MORE[0]);
    for (size_t i = 0; i < m; i++) {
        for (size_t j = 0; j < m; j++) {
            run(MORE[i], MORE[j]);
        }
    }

    /* Pseudo-random pairs across a wide but overflow-safe range. */
    for (int i = 0; i < 60; i++) {
        int32_t v1 = (int32_t)xr32();
        int32_t v2 = (int32_t)xr32();
        run(v1, v2);
    }

    return 0;
}
