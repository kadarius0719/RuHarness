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

static uint64_t g_rng_state = 0xC2B2AE3D27D4EB4FULL;

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

static void run_case(tflac_u32 blocksize, tflac_u32 channels, tflac_u32 bitdepth) {
    tflac_u32 r = max_size_frame(blocksize, channels, bitdepth);
    printf("case %d blocksize=%" PRIu32 " channels=%" PRIu32
           " bitdepth=%" PRIu32 " ret=%" PRIu32 "\n",
           g_case, blocksize, channels, bitdepth, r);
    g_case++;
}

int main(void) {
    static const tflac_u32 blocksize_vals[] = {
        0u, 1u, 2u, 7u, 8u, 16u, 255u, 256u, 4096u, 65535u, 65536u,
        1000000u, 4294967295u
    };
    static const tflac_u32 channels_vals[] = {
        0u, 1u, 2u, 3u, 4u, 8u, 255u, 4294967295u
    };
    static const tflac_u32 bitdepth_vals[] = {
        0u, 1u, 8u, 16u, 24u, 31u, 32u, 33u, 64u, 4294967295u
    };
    size_t nb = sizeof(blocksize_vals) / sizeof(blocksize_vals[0]);
    size_t nc = sizeof(channels_vals) / sizeof(channels_vals[0]);
    size_t nd = sizeof(bitdepth_vals) / sizeof(bitdepth_vals[0]);
    size_t i, j, k;

    for (i = 0; i < nb; i++) {
        for (j = 0; j < nc; j++) {
            for (k = 0; k < nd; k++) {
                run_case(blocksize_vals[i], channels_vals[j], bitdepth_vals[k]);
            }
        }
    }

    for (i = 0; i < 200; i++) {
        tflac_u32 bs = rand_u32();
        tflac_u32 ch = rand_u32();
        tflac_u32 bd = rand_u32();
        run_case(bs, ch, bd);
    }

    return 0;
}
