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

static uint64_t g_rng_state = 0x9E3779B97F4A7C15ULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static float rand_float(void) {
    uint32_t bits = (uint32_t)xorshift64();
    float f;
    memcpy(&f, &bits, sizeof(f));
    return f;
}

static int rand_int_range(int lo, int hi) {
    uint64_t r = xorshift64();
    uint64_t span = (uint64_t)((int64_t)hi - (int64_t)lo) + 1ULL;
    uint64_t v = r % span;
    return (int)((int64_t)lo + (int64_t)v);
}

static int g_case = 0;

static void run_case(float y, int exp_q2) {
    float r = ldexp_q2(y, exp_q2);
    printf("case %d y=%a exp_q2=%d ret=%a\n", g_case, y, exp_q2, r);
    g_case++;
}

int main(void) {
    static const float y_values[] = {
        0.0f, -0.0f, 1.0f, -1.0f, 0.5f, -0.5f, 2.0f, -2.0f,
        3.14159274f, -3.14159274f, 100.0f, -100.0f,
        FLT_MIN, -FLT_MIN, FLT_MAX, -FLT_MAX,
        0x1p-149f, -0x1p-149f,
        1.0e10f, -1.0e10f, 1.0e-10f, -1.0e-10f,
        INFINITY, -INFINITY, NAN
    };
    static const int small_exp_q2[] = {
        0, 1, 2, 3, 4, 5, 8, 16, 32, 60, 90, 100, 110, 118, 119
    };
    static const int moderate_exp_q2[] = {
        120, 121, 122, 150, 239, 240, 241, 480, 1000, 5000, 20000, 100000
    };
    size_t ny = sizeof(y_values) / sizeof(y_values[0]);
    size_t ns = sizeof(small_exp_q2) / sizeof(small_exp_q2[0]);
    size_t nm = sizeof(moderate_exp_q2) / sizeof(moderate_exp_q2[0]);
    size_t i, j;

    for (i = 0; i < ny; i++) {
        for (j = 0; j < ns; j++) {
            run_case(y_values[i], small_exp_q2[j]);
        }
    }
    for (i = 0; i < ny; i++) {
        for (j = 0; j < nm; j++) {
            run_case(y_values[i], moderate_exp_q2[j]);
        }
    }

    run_case(1.0f, INT_MAX);

    for (i = 0; i < 200; i++) {
        float y = rand_float();
        int e = rand_int_range(0, 100000);
        run_case(y, e);
    }

    return 0;
}
