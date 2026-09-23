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

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static int case_id = 0;

static void run_case(const char *label, const unsigned char *block64,
                      unsigned int mask, unsigned short init_max16,
                      unsigned short init_min16) {
    unsigned char block[64];
    unsigned short max16 = init_max16;
    unsigned short min16 = init_min16;
    int ret;
    memcpy(block, block64, 64);
    ret = refine_block(block, &max16, &min16, mask);
    printf("case %d %s mask=%08" PRIx32 " init_max=%04x init_min=%04x "
           "ret=%d final_max=%04x final_min=%04x\n",
           case_id++, label, (uint32_t)mask, (unsigned)init_max16,
           (unsigned)init_min16, ret, (unsigned)max16, (unsigned)min16);
}

static void run_solid_sweep(int v, const unsigned char *block64) {
    unsigned char block[64];
    unsigned short max16 = 0;
    unsigned short min16 = 0;
    int ret;
    memcpy(block, block64, 64);
    /* mask=0 satisfies (mask ^ (mask << 2)) < 4, forcing the low-variance
       "solid block" branch that indexes stb__OMatch5 / stb__OMatch6
       directly by the block's averaged r/g/b, so this sweep touches every
       row of both 256-entry tables (both matched columns, since column 0
       feeds final_max and column 1 feeds final_min). */
    ret = refine_block(block, &max16, &min16, 0u);
    printf("case %d solid_sweep v=%d ret=%d final_max=%04x final_min=%04x\n",
           case_id++, v, ret, (unsigned)max16, (unsigned)min16);
}

int main(void) {
    uint32_t rng = 741852963u;
    unsigned char solid_gray[64];
    unsigned char ramp[64];
    unsigned char extremes[64];
    unsigned char single_outlier[64];
    unsigned char rand_block[64];
    int i, r, v;

    for (i = 0; i < 16; i++) {
        solid_gray[i * 4 + 0] = 128;
        solid_gray[i * 4 + 1] = 128;
        solid_gray[i * 4 + 2] = 128;
        solid_gray[i * 4 + 3] = 255;
    }
    for (i = 0; i < 16; i++) {
        ramp[i * 4 + 0] = (unsigned char)(i * 17);
        ramp[i * 4 + 1] = (unsigned char)(255 - i * 17);
        ramp[i * 4 + 2] = (unsigned char)((i * 37) & 0xFF);
        ramp[i * 4 + 3] = 255;
    }
    for (i = 0; i < 16; i++) {
        int hi = (i % 2 == 0);
        extremes[i * 4 + 0] = (unsigned char)(hi ? 255 : 0);
        extremes[i * 4 + 1] = (unsigned char)(hi ? 255 : 0);
        extremes[i * 4 + 2] = (unsigned char)(hi ? 255 : 0);
        extremes[i * 4 + 3] = 255;
    }
    for (i = 0; i < 16; i++) {
        single_outlier[i * 4 + 0] = 100;
        single_outlier[i * 4 + 1] = 100;
        single_outlier[i * 4 + 2] = 100;
        single_outlier[i * 4 + 3] = 255;
    }
    single_outlier[0 * 4 + 0] = 250;
    single_outlier[0 * 4 + 1] = 10;
    single_outlier[0 * 4 + 2] = 5;

    unsigned int masks[6] = {0x00000000u, 0xFFFFFFFFu, 0x55555555u,
                              0xAAAAAAAAu, 0x1B4E7A93u, 0x0000FFFFu};
    unsigned short init_maxes[3] = {0x0000u, 0xFFFFu, 0x7BEFu};
    unsigned short init_mins[3] = {0x0000u, 0xFFFFu, 0x18E3u};

    const unsigned char *blocks[5];
    const char *block_names[5];
    blocks[0] = solid_gray; block_names[0] = "solid_gray";
    blocks[1] = ramp; block_names[1] = "ramp";
    blocks[2] = extremes; block_names[2] = "extremes";
    blocks[3] = single_outlier; block_names[3] = "single_outlier";

    for (int b = 0; b < 4; b++) {
        for (int mi = 0; mi < 6; mi++) {
            for (int ii = 0; ii < 3; ii++) {
                run_case(block_names[b], blocks[b], masks[mi],
                         init_maxes[ii], init_mins[ii]);
            }
        }
    }

    for (r = 0; r < 20; r++) {
        for (i = 0; i < 64; i++) rand_block[i] = (unsigned char)(xorshift32(&rng) & 0xFFu);
        unsigned int rmask = xorshift32(&rng);
        unsigned short rmax = (unsigned short)(xorshift32(&rng) & 0xFFFFu);
        unsigned short rmin = (unsigned short)(xorshift32(&rng) & 0xFFFFu);
        run_case("random", rand_block, rmask, rmax, rmin);
    }

    /* Exhaustive solid-block sweep: v = 0..255 with every pixel's r=g=b=v
       makes the averaged channel equal v exactly ((8 + 16*v) >> 4 == v),
       so this walks every index of stb__OMatch5 and stb__OMatch6 once,
       printing both table columns for each row via final_max/final_min.
       Any single mutated table constant changes exactly one of these
       255*... lines, killing the surviving table-element mutants. */
    for (v = 0; v < 256; v++) {
        unsigned char sweep_block[64];
        for (i = 0; i < 16; i++) {
            sweep_block[i * 4 + 0] = (unsigned char)v;
            sweep_block[i * 4 + 1] = (unsigned char)v;
            sweep_block[i * 4 + 2] = (unsigned char)v;
            sweep_block[i * 4 + 3] = 255;
        }
        run_solid_sweep(v, sweep_block);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
