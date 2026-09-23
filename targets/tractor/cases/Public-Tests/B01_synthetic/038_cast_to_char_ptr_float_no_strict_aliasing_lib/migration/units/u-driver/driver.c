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

static float bits_to_float(uint32_t bits) {
    float f;
    memcpy(&f, &bits, sizeof(f));
    return f;
}

static void run_case(int idx, float x) {
    printf("case %d x=%a ", idx, (double)x);
    driver(x);
}

int main(void) {
    static const uint32_t fixed_bits[] = {
        0x00000000u, 0x80000000u,
        0x3F800000u, 0xBF800000u,
        0x40000000u, 0xC0000000u,
        0x7F800000u, 0xFF800000u,
        0x7FC00000u, 0xFFC00000u,
        0x00000001u, 0x80000001u,
        0x007FFFFFu, 0x807FFFFFu,
        0x00800000u, 0x80800000u,
        0x7F7FFFFFu, 0xFF7FFFFFu,
        0x3F000000u, 0x42C80000u,
        0xC2C80000u, 0x4B000000u
    };
    const int n_fixed = (int)(sizeof(fixed_bits) / sizeof(fixed_bits[0]));
    int idx = 0;
    int i;

    for (i = 0; i < n_fixed; i++) {
        run_case(idx, bits_to_float(fixed_bits[i]));
        idx++;
    }

    rng_state = 2463534242u;
    for (i = 0; i < 64; i++) {
        uint32_t r = xorshift32();
        run_case(idx, bits_to_float(r));
        idx++;
    }

    return 0;
}
