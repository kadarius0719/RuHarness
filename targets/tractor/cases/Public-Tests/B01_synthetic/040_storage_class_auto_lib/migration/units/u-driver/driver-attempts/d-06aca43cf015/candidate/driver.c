#include "driver.h"

#include <stdio.h>
#include <stdint.h>

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
        0, 1, -1, 2, -2, 150, -150, 300, -300, 301, -301,
        1000, -1000, 1000000, -1000000,
        1073741673, -1073741824,
        1073741672, -1073741823,
        500000000, -500000000,
        1073000000, -1073000000
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
        uint32_t masked = r % 2147483498u;
        int v = (int)masked + (-1073741824);
        run_case(idx, v);
        idx++;
    }

    return 0;
}
