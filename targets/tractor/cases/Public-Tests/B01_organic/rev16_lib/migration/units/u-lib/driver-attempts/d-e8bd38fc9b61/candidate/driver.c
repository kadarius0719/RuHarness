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

static uint64_t g_rng_state = 0x27D4EB2F165667C5ULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static uint32_t rand_u32(void) {
    return (uint32_t)(xorshift64() & 0xFFFFFFFFULL);
}

static int g_case = 0;

static void run_case(uint32_t a) {
    uint32_t r = rev16(a);
    printf("case %d a=%" PRIu32 " ret=%" PRIu32 "\n", g_case, a, r);
    g_case++;
}

int main(void) {
    int i;
    static const uint32_t fixed_vals[] = {
        0x00000000u, 0xFFFFFFFFu, 0x00000001u, 0x00008000u, 0x0000FFFFu,
        0xFFFF0000u, 0x00000002u, 0x00004000u, 0x0000AAAAu, 0x00005555u,
        0x0000CCCCu, 0x00003333u, 0x0000F0F0u, 0x00000F0Fu, 0x0000FF00u,
        0x000000FFu, 0x00001234u, 0x0000ABCDu, 0xAAAA1234u, 0x5555ABCDu,
        0x12340000u, 0xFFFF1234u
    };
    size_t n = sizeof(fixed_vals) / sizeof(fixed_vals[0]);
    size_t j;

    for (j = 0; j < n; j++) {
        run_case(fixed_vals[j]);
    }

    /* exercise every individual bit position, including the upper 16
       bits that the unit must discard */
    for (i = 0; i < 32; i++) {
        run_case((uint32_t)1u << i);
        run_case(~((uint32_t)1u << i));
    }

    for (j = 0; j < 400; j++) {
        run_case(rand_u32());
    }

    return 0;
}
