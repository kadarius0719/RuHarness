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

static uint8_t g_data[320];
static uint32_t xr_state;
static int g_case = 0;

static uint32_t xr32(void) {
    uint32_t x = xr_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    xr_state = x;
    return x;
}

static void run(const uint8_t *d, uint32_t len, uint16_t seed, const char *tag) {
    tflac_u16 r = crc16(d, (tflac_u32)len, (tflac_u16)seed);
    g_case++;
    printf("case %d %s len=%" PRIu32 " seed=%" PRIu32 " crc=%" PRIu32 "\n",
           g_case, tag, len, (uint32_t)seed, (uint32_t)r);
}

int main(void) {
    xr_state = 0x1234ABCDu;

    /* deterministic byte pattern with good bit spread */
    for (size_t i = 0; i < sizeof(g_data); i++) {
        g_data[i] = (uint8_t)(((i * 31u) + 17u) ^ (i >> 3));
    }

    static const uint32_t lengths[] = {
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 16, 17,
        23, 24, 31, 32, 33, 63, 64, 65, 100, 255, 256, 300
    };
    size_t nlen = sizeof(lengths) / sizeof(lengths[0]);

    static const uint16_t seeds[] = {0x0000, 0xFFFF, 0x1234, 0xABCD};
    size_t nseed = sizeof(seeds) / sizeof(seeds[0]);

    for (size_t i = 0; i < nlen; i++) {
        for (size_t s = 0; s < nseed; s++) {
            run(g_data, lengths[i], seeds[s], "fixed");
        }
    }

    /* pure zero and pure 0xFF buffers, a few lengths */
    {
        static uint8_t zeros[64];
        static uint8_t ones[64];
        memset(zeros, 0x00, sizeof(zeros));
        memset(ones, 0xFF, sizeof(ones));
        uint32_t special_lens[] = {0, 1, 7, 8, 9, 63, 64};
        size_t n = sizeof(special_lens) / sizeof(special_lens[0]);
        for (size_t i = 0; i < n; i++) {
            run(zeros, special_lens[i], 0x0000, "zeros");
            run(ones, special_lens[i], 0xFFFF, "ones");
        }
    }

    /* pseudo-random content and pseudo-random (but bounded, buffer-safe) length/seed */
    {
        uint8_t rnd[280];
        for (size_t i = 0; i < sizeof(rnd); i++) {
            rnd[i] = (uint8_t)(xr32() & 0xFFu);
        }
        for (int i = 0; i < 20; i++) {
            uint32_t len = xr32() % (uint32_t)(sizeof(rnd) + 1u);
            uint16_t seed = (uint16_t)(xr32() & 0xFFFFu);
            run(rnd, len, seed, "random");
        }
    }

    return 0;
}
