#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

static uint32_t rng_state = 0x13579bdfu;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static int g_case_id = 1;

static void run_case(uint16_t h) {
    float f = half2float(h);
    printf("case %d h=%" PRIu16 " ret=%a\n", g_case_id, h, (double)f);
    g_case_id++;
}

int main(void) {
    int n, lo;
    uint32_t i;

    /* Named edge cases: +0, -0, smallest subnormal half, largest
       subnormal half, smallest normal half, largest normal half,
       +inf, -inf, qNaN, 1.0, -1.0. */
    run_case(0x0000u);
    run_case(0x8000u);
    run_case(0x0001u);
    run_case(0x03ffu);
    run_case(0x0400u);
    run_case(0x7bffu);
    run_case(0x7c00u);
    run_case(0xfc00u);
    run_case(0x7e00u);
    run_case(0x3c00u);
    run_case(0xbc00u);

    /* Full sweep of the mantissa-table's two halves: n=1 selects the
       offset=1024 half (mantissa indices 1024..2047), n=32 selects the
       offset=0 half (indices 0..1023). Together this exercises every
       entry of the 2048-entry mantissa lookup table exactly once. */
    for (lo = 0; lo < 1024; ++lo) {
        uint16_t h1 = (uint16_t)((1 << 10) | lo);
        run_case(h1);
    }
    for (lo = 0; lo < 1024; ++lo) {
        uint16_t h2 = (uint16_t)((32 << 10) | lo);
        run_case(h2);
    }

    /* Sweep every value of n (0..63) to exercise every entry of the
       64-entry exponent and offset tables, at a few fixed mantissa
       low-bit patterns. */
    for (n = 0; n < 64; ++n) {
        uint16_t ha = (uint16_t)((n << 10) | 0x000);
        uint16_t hb = (uint16_t)((n << 10) | 0x001);
        uint16_t hc = (uint16_t)((n << 10) | 0x3ff);
        run_case(ha);
        run_case(hb);
        run_case(hc);
    }

    /* Pseudo-random half-precision bit patterns. */
    for (i = 0; i < 40; ++i) {
        uint16_t h = (uint16_t)(xorshift32() & 0xffffu);
        run_case(h);
    }

    return 0;
}
