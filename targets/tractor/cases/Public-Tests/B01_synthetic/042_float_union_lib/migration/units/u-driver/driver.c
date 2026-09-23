#include "driver.h"

#include <stdio.h>
#include <stdint.h>
#include <string.h>

static uint32_t rng_state;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static uint64_t xorshift64_pair(void) {
    uint64_t hi = (uint64_t)xorshift32();
    uint64_t lo = (uint64_t)xorshift32();
    return (hi << 32) | lo;
}

static double bits_to_double(uint64_t bits) {
    double d;
    memcpy(&d, &bits, sizeof(d));
    return d;
}

static void run_case(int idx, double f) {
    printf("case %d ", idx);
    driver(f);
}

int main(void) {
    static const uint64_t fixed_bits[] = {
        0x0000000000000000ULL, 0x8000000000000000ULL,
        0x3FF0000000000000ULL, 0xBFF0000000000000ULL,
        0x4000000000000000ULL, 0xC000000000000000ULL,
        0x7FF0000000000000ULL, 0xFFF0000000000000ULL,
        0x7FF8000000000000ULL, 0xFFF8000000000000ULL,
        0x0000000000000001ULL, 0x8000000000000001ULL,
        0x000FFFFFFFFFFFFFULL, 0x800FFFFFFFFFFFFFULL,
        0x0010000000000000ULL, 0x8010000000000000ULL,
        0x7FEFFFFFFFFFFFFFULL, 0xFFEFFFFFFFFFFFFFULL,
        0x3FE0000000000000ULL, 0x400921FB54442D18ULL,
        0x4005BF0A8B145769ULL, 0xC059000000000000ULL,
        0x4059000000000000ULL
    };
    const int n_fixed = (int)(sizeof(fixed_bits) / sizeof(fixed_bits[0]));
    int idx = 0;
    int i;

    for (i = 0; i < n_fixed; i++) {
        run_case(idx, bits_to_double(fixed_bits[i]));
        idx++;
    }

    rng_state = 2463534242u;
    for (i = 0; i < 64; i++) {
        uint64_t r = xorshift64_pair();
        run_case(idx, bits_to_double(r));
        idx++;
    }

    return 0;
}
