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

static uint32_t xs32(uint32_t *state) {
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

static float rand_float(uint32_t *state, float lo, float hi) {
    uint32_t r = xs32(state);
    float t = (float)r / (float)UINT32_MAX;
    return lo + t * (hi - lo);
}

static void run_case(int *case_no, const float *src) {
    float dest[3];
    dest[0] = 0.0f;
    dest[1] = 0.0f;
    dest[2] = 0.0f;
    rgb_to_hsv(dest, src);
    printf("case %d h=%a s=%a v=%a\n", *case_no, (double)dest[0],
           (double)dest[1], (double)dest[2]);
    (*case_no)++;
}

int main(void) {
    int case_no = 0;

    static const float fixed_cases[][3] = {
        {0.0f, 0.0f, 0.0f},
        {0.5f, 0.5f, 0.5f},
        {1.0f, 1.0f, 1.0f},
        {1.0f, 0.0f, 0.0f},
        {0.0f, 1.0f, 0.0f},
        {0.0f, 0.0f, 1.0f},
        {1.0f, 1.0f, 0.0f},
        {1.0f, 0.0f, 1.0f},
        {0.0f, 1.0f, 1.0f},
        {0.2f, 0.6f, 0.9f},
        {0.9f, 0.6f, 0.2f},
        {0.6f, 0.9f, 0.2f},
        {0.9f, 0.2f, 0.6f},
        {0.2f, 0.9f, 0.6f},
        {0.6f, 0.2f, 0.9f},
        {-1.0f, 0.5f, 0.25f},
        {2.0f, 1.0f, 0.5f},
        {1.0f, 2.0f, 0.5f},
        {0.5f, 1.0f, 2.0f},
        {-0.5f, -0.25f, -0.75f},
        {FLT_MAX, 0.0f, 0.0f},
        {0.0f, FLT_MAX, 0.0f},
        {0.0f, 0.0f, FLT_MAX},
        {-FLT_MAX, -FLT_MAX, -FLT_MAX},
        {FLT_MIN, FLT_MIN, FLT_MIN},
        {FLT_MIN, 2.0f * FLT_MIN, 3.0f * FLT_MIN},
        {100.0f, 50.0f, 25.0f},
        {25.0f, 100.0f, 50.0f},
        {50.0f, 25.0f, 100.0f},
        {0.10000001f, 0.1f, 0.09999999f},
    };
    size_t n_fixed = sizeof(fixed_cases) / sizeof(fixed_cases[0]);
    for (size_t i = 0; i < n_fixed; i++) {
        run_case(&case_no, fixed_cases[i]);
    }

    uint32_t state = 2463534242u;
    for (int i = 0; i < 300; i++) {
        float src[3];
        src[0] = rand_float(&state, -2.0f, 2.0f);
        src[1] = rand_float(&state, -2.0f, 2.0f);
        src[2] = rand_float(&state, -2.0f, 2.0f);
        run_case(&case_no, src);
    }

    for (int i = 0; i < 100; i++) {
        float src[3];
        src[0] = rand_float(&state, 0.0f, 1.0f);
        src[1] = rand_float(&state, 0.0f, 1.0f);
        src[2] = rand_float(&state, 0.0f, 1.0f);
        run_case(&case_no, src);
    }

    return 0;
}
