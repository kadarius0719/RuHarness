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

static void run_case(const char *label, tflac_u32 bitdepth, tflac_u32 blocksize,
                      tflac_u32 channel, tflac_u32 stride) {
    tflac t;
    memset(&t, 0, sizeof t);
    t.bitdepth = bitdepth;
    t.cur_blocksize = blocksize;
    decorrelate(&t, channel, stride);
    printf("case %d %s bitdepth=%" PRIu32 " blocksize=%" PRIu32
           " channel=%" PRIu32 " stride=%" PRIu32
           " subframe_bitdepth=%" PRIu32 " constant=%u"
           " residual_errors0=%" PRIu64
           " residuals=%" PRId32 ",%" PRId32 ",%" PRId32 ",%" PRId32 ",%" PRId32 "\n",
           case_id++, label, bitdepth, blocksize, channel, stride,
           t.subframe_bitdepth, (unsigned)t.constant,
           t.residual_errors[0],
           t.residuals[0], t.residuals[1], t.residuals[2], t.residuals[3], t.residuals[4]);
}

int main(void) {
    uint32_t rng = 192837465u;
    int r;

    run_case("chan0_stride0_bs0", 16, 0, 0, 0);
    run_case("chan0_stride0_bs5", 16, 5, 0, 0);
    run_case("chan0_stride1_bs5", 16, 5, 0, 1);
    run_case("chan0_stride7_bs3", 20, 3, 0, 7);
    run_case("chan0_stride_max_bs5", 24, 5, 0, 0xFFFFFFFFu);
    run_case("chan0_stride_minflag_bs5", 8, 5, 0, 0x80000000u);
    run_case("chan1_stride0_bs5", 16, 5, 1, 0);
    run_case("chan1_stride1_bs5", 16, 5, 1, 1);
    run_case("chan1_stride_max_bs5", 24, 5, 1, 0xFFFFFFFFu);
    run_case("chan1_stride_minflag_bs5", 8, 5, 1, 0x80000000u);
    run_case("chan_other_bs4", 12, 4, 42, 123);
    run_case("chan0_bs1", 16, 1, 0, 999);
    run_case("chan0_bs2_stride5", 16, 2, 0, 5);
    run_case("bitdepth0", 0, 5, 0, 3);
    run_case("bitdepth_max", 0xFFFFFFFFu, 5, 0, 3);
    run_case("bitdepth_max_chan1", 0xFFFFFFFFu, 5, 1, 3);
    run_case("chan_max_val", 16, 5, 0xFFFFFFFFu, 3);
    run_case("bs4_stride2", 10, 4, 0, 2);
    run_case("bs5_stride3_chan2", 10, 5, 2, 3);
    run_case("bs0_stride_max", 10, 0, 0, 0xFFFFFFFFu);

    for (r = 0; r < 20; r++) {
        tflac_u32 bd = xorshift32(&rng) % 33u;
        tflac_u32 bs = xorshift32(&rng) % 6u;
        tflac_u32 ch = xorshift32(&rng) % 4u;
        tflac_u32 st = xorshift32(&rng);
        run_case("random", bd, bs, ch, st);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
