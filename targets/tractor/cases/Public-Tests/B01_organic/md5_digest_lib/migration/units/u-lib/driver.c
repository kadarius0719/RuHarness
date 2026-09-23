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

static uint64_t g_rng_state = 0xA0761D6478BD642FULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static tflac_u32 rand_u32(void) {
    return (tflac_u32)(xorshift64() & 0xFFFFFFFFULL);
}

static int g_case = 0;

static void run_case(tflac_u32 a, tflac_u32 b, tflac_u32 c, tflac_u32 d) {
    tflac_md5 m;
    tflac_u8 out[16];
    size_t i;

    m.a = a;
    m.b = b;
    m.c = c;
    m.d = d;
    memset(out, 0, sizeof(out));

    md5_digest(&m, out);

    printf("case %d a=%" PRIu32 " b=%" PRIu32 " c=%" PRIu32 " d=%" PRIu32
           " out=",
           g_case, a, b, c, d);
    for (i = 0; i < sizeof(out); i++) {
        printf("%02" PRIx8, out[i]);
    }
    printf("\n");
    g_case++;
}

int main(void) {
    static const tflac_u32 vals[] = {
        0x00000000u, 0x00000001u, 0xFFFFFFFFu, 0x80000000u, 0x7FFFFFFFu,
        0x12345678u, 0x89ABCDEFu, 0xDEADBEEFu, 0x01020304u, 0xAAAAAAAAu,
        0x55555555u
    };
    size_t nv = sizeof(vals) / sizeof(vals[0]);
    size_t i, j;

    /* Full 4-D sweep would be too large; walk the diagonal plus every
       pairwise combination against a fixed baseline to exercise every
       byte of every field, then finish with a broad random sweep. */
    for (i = 0; i < nv; i++) {
        run_case(vals[i], vals[i], vals[i], vals[i]);
    }
    for (i = 0; i < nv; i++) {
        for (j = 0; j < nv; j++) {
            run_case(vals[i], vals[j], 0x0F0F0F0Fu, 0xF0F0F0F0u);
            run_case(0x0F0F0F0Fu, 0xF0F0F0F0u, vals[i], vals[j]);
        }
    }

    for (i = 0; i < 300; i++) {
        tflac_u32 a = rand_u32();
        tflac_u32 b = rand_u32();
        tflac_u32 c = rand_u32();
        tflac_u32 d = rand_u32();
        run_case(a, b, c, d);
    }

    return 0;
}
