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

tflac_u32 tflac_size_memory(tflac_u32 blocksize);

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

static void run_validate(const char *tag, tflac_u32 blocksize, tflac_u32 samplerate,
                          tflac_u32 channels, tflac_u32 bitdepth, tflac_u8 channel_mode,
                          tflac_u8 max_rice_value, tflac_u8 min_partition_order,
                          tflac_u8 max_partition_order) {
    tflac t;
    memset(&t, 0, sizeof(t));
    t.blocksize = blocksize;
    t.samplerate = samplerate;
    t.channels = channels;
    t.bitdepth = bitdepth;
    t.channel_mode = channel_mode;
    t.max_rice_value = max_rice_value;
    t.min_partition_order = min_partition_order;
    t.max_partition_order = max_partition_order;

    int ret = flac_validate(&t);
    g_case++;
    printf("case %d %s ret=%d channel_mode=%" PRIu32 " max_rice_value=%" PRIu32
           " partition_order=%" PRIu32 " cur_blocksize=%" PRIu32 "\n",
           g_case, tag, ret, (uint32_t)t.channel_mode, (uint32_t)t.max_rice_value,
           (uint32_t)t.partition_order, (uint32_t)t.cur_blocksize);
}

static void run_size_memory(const char *tag, tflac_u32 blocksize) {
    tflac_u32 r = tflac_size_memory(blocksize);
    g_case++;
    printf("case %d %s blocksize=%" PRIu32 " size=%" PRIu32 "\n", g_case, tag, blocksize, r);
}

int main(void) {
    xr_state = 0x55AA55AAu;

    /* Each early-return branch, isolated by keeping every earlier check valid. */
    run_validate("blocksize_too_small", 15, 44100, 2, 16, 0, 14, 0, 6);
    run_validate("blocksize_too_large", 65536, 44100, 2, 16, 0, 14, 0, 6);
    run_validate("samplerate_zero", 4096, 0, 2, 16, 0, 14, 0, 6);
    run_validate("samplerate_too_large", 4096, 655351, 2, 16, 0, 14, 0, 6);
    run_validate("channels_zero", 4096, 44100, 0, 16, 0, 14, 0, 6);
    run_validate("channels_too_many", 4096, 44100, 9, 16, 0, 14, 0, 6);
    run_validate("bitdepth_zero", 4096, 44100, 2, 0, 0, 14, 0, 6);
    run_validate("bitdepth_too_large", 4096, 44100, 2, 33, 0, 14, 0, 6);
    run_validate("max_rice_too_large", 4096, 44100, 2, 16, 0, 31, 0, 6);
    run_validate("max_partition_too_large", 4096, 44100, 2, 16, 0, 14, 0, 16);
    run_validate("min_gt_max_partition", 4096, 44100, 2, 16, 0, 14, 7, 6);

    /* Boundary values that are exactly still valid. */
    run_validate("blocksize_min_ok", 16, 44100, 2, 16, 0, 14, 0, 6);
    run_validate("blocksize_max_ok", 65535, 44100, 2, 16, 0, 14, 0, 15);
    run_validate("samplerate_max_ok", 4096, 655350, 2, 16, 0, 14, 0, 6);
    run_validate("channels_max_ok", 4096, 44100, 8, 16, 0, 14, 0, 6);
    run_validate("bitdepth_max_ok", 4096, 44100, 2, 32, 0, 14, 0, 6);
    run_validate("max_rice_max_ok", 4096, 44100, 2, 16, 0, 30, 0, 6);
    run_validate("max_partition_max_ok", 4096, 44100, 2, 16, 0, 14, 0, 15);
    run_validate("min_eq_max_partition", 4096, 44100, 2, 16, 0, 14, 6, 6);

    /* channel_mode forcing logic. */
    run_validate("channel_mode_kept", 4096, 44100, 2, 16, 1, 14, 0, 6);
    run_validate("channel_mode_forced_channels", 4096, 44100, 3, 16, 1, 14, 0, 6);
    run_validate("channel_mode_forced_bitdepth32", 4096, 44100, 2, 32, 3, 14, 0, 6);
    run_validate("channel_mode_already_independent", 4096, 44100, 2, 16, 0, 14, 0, 6);

    /* max_rice_value auto-selection. */
    run_validate("rice_auto_low_bitdepth", 4096, 44100, 2, 16, 0, 0, 0, 6);
    run_validate("rice_auto_high_bitdepth", 4096, 44100, 2, 24, 0, 0, 0, 6);
    run_validate("rice_explicit_kept", 4096, 44100, 2, 16, 0, 20, 0, 6);

    /* partition_order loop over various blocksize divisibility patterns. */
    run_validate("partition_power_of_two", 4096, 44100, 2, 16, 0, 14, 0, 6);
    run_validate("partition_odd_blocksize", 17, 44100, 2, 16, 0, 14, 0, 6);
    run_validate("partition_min_eq_max", 4096, 44100, 2, 16, 0, 14, 6, 6);
    run_validate("partition_full_range", 32768, 44100, 2, 16, 0, 14, 0, 15);
    run_validate("partition_min_blocksize", 16, 44100, 2, 16, 0, 14, 0, 15);
    run_validate("partition_prime_ish", 4095, 44100, 2, 16, 0, 14, 0, 15);

    /* pseudo-random valid-envelope combinations */
    for (int i = 0; i < 20; i++) {
        tflac_u32 blocksize = 16u + (xr32() % (65535u - 16u + 1u));
        tflac_u32 samplerate = 1u + (xr32() % 655350u);
        tflac_u32 channels = 1u + (xr32() % 8u);
        tflac_u32 bitdepth = 1u + (xr32() % 32u);
        tflac_u8 channel_mode = (tflac_u8)(xr32() % 4u);
        tflac_u8 max_rice_value = (tflac_u8)(xr32() % 31u);
        tflac_u8 max_partition_order = (tflac_u8)(xr32() % 16u);
        tflac_u8 min_partition_order = (tflac_u8)(xr32() % ((uint32_t)max_partition_order + 1u));
        run_validate("random", blocksize, samplerate, channels, bitdepth, channel_mode,
                     max_rice_value, min_partition_order, max_partition_order);
    }

    /* tflac_size_memory: boundaries and a range of representative sizes. */
    run_size_memory("zero", 0);
    run_size_memory("one", 1);
    run_size_memory("blocksize_min", 16);
    run_size_memory("blocksize_common", 4096);
    run_size_memory("blocksize_max_valid", 65535);
    run_size_memory("just_above_valid", 65536);
    run_size_memory("mid_large", 1000000);
    run_size_memory("near_uint32_max", 0xFFFFFFFEu);
    run_size_memory("uint32_max", 0xFFFFFFFFu);
    run_size_memory("aligned_pattern", 0xFFFFFFF0u);
    run_size_memory("power_of_two", 32768);

    for (int i = 0; i < 15; i++) {
        tflac_u32 v = xr32();
        run_size_memory("random", v);
    }

    return 0;
}
