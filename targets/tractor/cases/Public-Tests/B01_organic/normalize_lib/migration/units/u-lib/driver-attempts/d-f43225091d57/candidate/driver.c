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

#define MAX_N 64

static uint64_t g_rng_state = 0x9E3779B97F4A7C15ULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static float rand_bits_float(void) {
    uint32_t bits = (uint32_t)xorshift64();
    float f;
    memcpy(&f, &bits, sizeof(f));
    return f;
}

static int g_case = 0;

static void print_vec(const char *mode, const char *gen, int size,
                       const float *v) {
    int i;
    printf("case %d mode=%s gen=%s size=%d dest=[", g_case, mode, gen, size);
    for (i = 0; i < size; i++) {
        if (i != 0) {
            printf(" ");
        }
        printf("%a", v[i]);
    }
    printf("]\n");
    g_case++;
}

static void gen_zero(float *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i] = 0.0f;
    }
}

static void gen_normal(float *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i] = ((float)(i % 7) - 3.0f) * 1.5f + 0.25f;
    }
}

static void gen_large(float *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i] = (i == 0) ? FLT_MAX : (float)(i);
    }
}

static void gen_nan(float *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i] = (i == n / 2) ? NAN : (float)(i + 1);
    }
}

static void gen_random_bits(float *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        float f = rand_bits_float();
        if (f != f) {
            f = 1.0f;
        }
        buf[i] = f;
    }
}

static void gen_subnormal(float *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i] = 0x1p-148f * (float)(i + 1);
    }
}

typedef void (*gen_fn)(float *, int);

int main(void) {
    static const int sizes[] = { 0, 1, 2, 3, 4, 5, 8, 16, 64 };
    static gen_fn gens[] = {
        gen_zero, gen_normal, gen_large, gen_nan, gen_random_bits,
        gen_subnormal
    };
    static const char *gen_names[] = {
        "zero", "normal", "large", "nan", "randbits", "subnorm"
    };
    size_t nsizes = sizeof(sizes) / sizeof(sizes[0]);
    size_t ngens = sizeof(gens) / sizeof(gens[0]);
    size_t si, gi;
    static float src[MAX_N];
    static float dest[MAX_N];

    for (si = 0; si < nsizes; si++) {
        int n = sizes[si];
        for (gi = 0; gi < ngens; gi++) {
            gens[gi](src, n);
            memcpy(dest, src, (size_t)n * sizeof(float));
            normalize(dest, src, n);
            print_vec("outofplace", gen_names[gi], n, dest);

            gens[gi](src, n);
            normalize(src, src, n);
            print_vec("inplace", gen_names[gi], n, src);
        }
    }

    return 0;
}
