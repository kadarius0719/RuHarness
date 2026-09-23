#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

static uint32_t rng_state = 0x2468ace0u;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static float rand_range(float lo, float hi) {
    uint32_t r = xorshift32();
    float u = (float)(r >> 8) / (float)(1u << 24); /* [0,1) */
    return lo + u * (hi - lo);
}

static int g_case_id = 1;

static void run_case(float h, float s, float l) {
    float src[3];
    float dest[3];
    src[0] = h;
    src[1] = s;
    src[2] = l;
    dest[0] = 0.0f;
    dest[1] = 0.0f;
    dest[2] = 0.0f;
    hsl_to_rgb(dest, src);
    printf("case %d h=%a s=%a l=%a r=%a g=%a b=%a\n", g_case_id, (double)h,
           (double)s, (double)l, (double)dest[0], (double)dest[1],
           (double)dest[2]);
    g_case_id++;
}

int main(void) {
    static const float hues[] = {
        0.0f,    59.9f,   60.0f,   60.1f,   119.9f,  120.0f,  120.1f,
        179.9f,  180.0f,  180.1f,  239.9f,  240.0f,  240.1f,  299.9f,
        300.0f,  300.1f,  359.9f,  360.0f,  360.1f,  400.0f,  720.0f,
        -0.1f,   -1.0f,   -10.0f,  -59.0f,  -60.0f,  -61.0f,  -119.0f,
        -120.0f, -121.0f, -180.0f, -200.0f, -359.0f, -360.0f, -400.0f,
        -720.0f};
    size_t n = sizeof(hues) / sizeof(hues[0]);
    size_t hi;
    int i;

    /* s == 0 short-circuits to (l, l, l) regardless of h. */
    run_case(0.0f, 0.0f, 0.0f);
    run_case(123.0f, 0.0f, 0.5f);
    run_case(-45.0f, 0.0f, 1.0f);
    run_case(200.0f, 0.0f, -0.3f);
    run_case(10.0f, 0.0f, 2.0f);

    /* Sweep every branch boundary of the h-driven switch, including
       negative hues: the third else-if ("h < 120 && h < 180") is only
       ever reachable for h < 0 once the first two branches already
       ruled out h in [0, 120), so negative hues fall into that branch
       instead of the intended [120, 180) one -- worth pinning exactly. */
    for (hi = 0; hi < n; ++hi) {
        run_case(hues[hi], 0.5f, 0.5f);
    }

    /* Extreme / out-of-range saturation and lightness values. */
    run_case(30.0f, 1.0f, 0.0f);
    run_case(90.0f, 1.0f, 1.0f);
    run_case(150.0f, 1.0f, 0.5f);
    run_case(210.0f, -0.5f, 0.5f);
    run_case(270.0f, 2.0f, 0.5f);
    run_case(330.0f, 0.5f, -1.0f);
    run_case(45.0f, 0.5f, 2.0f);

    for (i = 0; i < 30; ++i) {
        float h = rand_range(-720.0f, 720.0f);
        float s = rand_range(-1.0f, 2.0f);
        float l = rand_range(-1.0f, 2.0f);
        run_case(h, s, l);
    }

    return 0;
}
