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

static uint64_t g_rng_state = 0xD1B54A32D192ED03ULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static int g_case = 0;

static void run_seed(uint64_t s0, uint64_t s1, int iterations) {
    cn_rnd_t rnd;
    int i;

    rnd.state[0] = s0;
    rnd.state[1] = s1;

    printf("case %d seed0=%" PRIu64 " seed1=%" PRIu64 "\n", g_case, s0, s1);
    for (i = 0; i < iterations; i++) {
        double v = next_double(&rnd);
        printf("case %d iter %d val=%a state0=%" PRIu64 " state1=%" PRIu64
               "\n",
               g_case, i, v, rnd.state[0], rnd.state[1]);
    }
    g_case++;
}

int main(void) {
    static const uint64_t seeds0[] = {
        0x0000000000000000ULL, 0x0000000000000001ULL,
        0xFFFFFFFFFFFFFFFFULL, 0x00000000FFFFFFFFULL,
        0x123456789ABCDEF0ULL, 0x8000000000000000ULL,
        0x0000000000000001ULL, 0xAAAAAAAAAAAAAAAAULL,
        0x5555555555555555ULL, 0x0123456789ABCDEFULL
    };
    static const uint64_t seeds1[] = {
        0x0000000000000000ULL, 0x0000000000000000ULL,
        0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFF00000000ULL,
        0xFEDCBA9876543210ULL, 0x0000000000000001ULL,
        0x8000000000000000ULL, 0x5555555555555555ULL,
        0xAAAAAAAAAAAAAAAAULL, 0xFEDCBA9876543210ULL
    };
    size_t n = sizeof(seeds0) / sizeof(seeds0[0]);
    size_t i;

    for (i = 0; i < n; i++) {
        run_seed(seeds0[i], seeds1[i], 15);
    }

    for (i = 0; i < 10; i++) {
        uint64_t s0 = xorshift64();
        uint64_t s1 = xorshift64();
        run_seed(s0, s1, 10);
    }

    return 0;
}
