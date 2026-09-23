#include "driver.h"

#include <stdio.h>
#include <stdint.h>
#include <limits.h>

static uint32_t rng_state;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static void run_case(int idx, int x) {
    printf("case %d x=%d ", idx, x);
    driver(x);
}

int main(void) {
    static const int fixed_vals[] = {
        0, 1, -1, 2, -2, 3, -3, 10, -10, 100, -100,
        127, -127, 128, -128, 255, -255, 256, -256,
        32767, -32767, 32768, -32768, 65535, -65535, 65536, -65536,
        1000000, -1000000, 123456789, -123456789,
        INT_MAX, INT_MIN, INT_MAX - 1, INT_MIN + 1
    };
    const int n_fixed = (int)(sizeof(fixed_vals) / sizeof(fixed_vals[0]));
    int idx = 0;
    int i;

    for (i = 0; i < n_fixed; i++) {
        run_case(idx, fixed_vals[i]);
        idx++;
    }

    rng_state = 2463534242u;
    for (i = 0; i < 64; i++) {
        uint32_t r = xorshift32();
        int v = (int)r;
        run_case(idx, v);
        idx++;
    }

    return 0;
}
