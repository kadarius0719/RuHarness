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

#define MAXCOUNT 64

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

static void print_batch(int *case_no, const float *dest, int count) {
    int i;
    for (i = 0; i < count; i++) {
        printf("case %d out %d x=%a y=%a\n", *case_no, i,
               (double)dest[2 * i], (double)dest[2 * i + 1]);
    }
    (*case_no)++;
}

int main(void) {
    int case_no = 0;
    float dummy_dest[1];
    float dummy_src[1];

    tfm(dummy_dest, dummy_src, 0);
    printf("case %d count=0\n", case_no);
    case_no++;

    tfm(dummy_dest, dummy_src, -5);
    printf("case %d count=-5\n", case_no);
    case_no++;

    tfm(dummy_dest, dummy_src, -1000000);
    printf("case %d count=-1000000\n", case_no);
    case_no++;

    static const float fixed[][3] = {
        {0.0f, 0.0f, 0.0f},
        {1.0f, 0.0f, 0.0f},
        {0.0f, 1.0f, 0.0f},
        {5.0f, 5.0f, 3.0f},
        {-5.0f, -3.0f, 2.0f},
        {-3.0f, -5.0f, 2.0f},
        {1000.0f, -1000.0f, 500.0f},
        {-1000.0f, 1000.0f, -500.0f},
        {0.0f, 0.0f, 1.0e6f},
        {1.0e6f, 1.0e6f, 0.0f},
        {1.0e6f, -1.0e6f, 1.0e6f},
        {FLT_MIN, -FLT_MIN, FLT_MIN},
        {0.1f, 0.2f, 0.05f},
        {0.2f, 0.1f, 0.05f},
        {-0.0f, 0.0f, 0.0f},
        {0.0f, -0.0f, 0.0f},
        {3.0f, 3.0f, 0.0f},
        {3.0f, 3.0f, -100.0f},
        {-100.0f, -100.0f, 0.0f},
        {100.0f, 99.999999f, 1.0f},
        {99.999999f, 100.0f, 1.0f},
    };
    size_t n_fixed = sizeof(fixed) / sizeof(fixed[0]);
    size_t i;
    for (i = 0; i < n_fixed; i++) {
        float src3[3];
        float dest2[2];
        src3[0] = fixed[i][0];
        src3[1] = fixed[i][1];
        src3[2] = fixed[i][2];
        tfm(dest2, src3, 1);
        print_batch(&case_no, dest2, 1);
    }

    {
        int lens[] = {2, 3, 7, 16, 32, 64};
        size_t n_lens = sizeof(lens) / sizeof(lens[0]);
        size_t li;
        uint32_t state = 2463534242u;
        for (li = 0; li < n_lens; li++) {
            int len = lens[li];
            float src[MAXCOUNT * 3];
            float dest[MAXCOUNT * 2];
            int k;
            for (k = 0; k < len * 3; k++) {
                src[k] = rand_float(&state, -50.0f, 50.0f);
            }
            tfm(dest, src, len);
            print_batch(&case_no, dest, len);
        }
    }

    {
        uint32_t state = 998877665u;
        int trial;
        for (trial = 0; trial < 40; trial++) {
            float src[3];
            float dest[2];
            src[0] = rand_float(&state, -1000.0f, 1000.0f);
            src[1] = rand_float(&state, -1000.0f, 1000.0f);
            src[2] = rand_float(&state, -1000.0f, 1000.0f);
            tfm(dest, src, 1);
            print_batch(&case_no, dest, 1);
        }
    }

    return 0;
}
