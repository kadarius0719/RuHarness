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

static uint32_t xs32(uint32_t *state) {
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

static void run_case(int *case_no, tflac_u32 samplerate, tflac_u32 channels,
                      tflac_u32 bitdepth, tflac_u8 channel_mode,
                      tflac_u32 cur_blocksize) {
    tflac t;
    memset(&t, 0, sizeof(t));
    t.samplerate = samplerate;
    t.channels = channels;
    t.bitdepth = bitdepth;
    t.channel_mode = channel_mode;
    t.cur_blocksize = cur_blocksize;
    update_frame_header(&t);
    printf("case %d sr=%" PRIu32 " ch=%" PRIu32 " bd=%" PRIu32
           " mode=%" PRIu8 " bs=%" PRIu32 " fh=0x%08" PRIX32 "\n",
           *case_no, samplerate, channels, bitdepth, channel_mode,
           cur_blocksize, t.frame_header);
    (*case_no)++;
}

int main(void) {
    int case_no = 0;
    tflac_u32 base_sr = 44100;
    tflac_u32 base_ch = 2;
    tflac_u32 base_bd = 16;
    tflac_u32 base_bs = 4096;
    tflac_u8 base_mode = 0;

    static const tflac_u32 bs_values[] = {
        192,  576,   1152,  2304,   4608,  256,   512,     1024,
        2048, 4096,  8192,  16384,  32768, 100,   0,       1,
        255,  300,   5000,  100000, 4294967295u};
    size_t n_bs = sizeof(bs_values) / sizeof(bs_values[0]);

    static const tflac_u32 sr_values[] = {
        882000, 176400, 192000, 8000,   16000,  22050,  24000,
        32000,  44100,  48000,  96000,  5000,   255000, 256000,
        12345,  655350, 655360, 70001,  0,      4294967295u};
    size_t n_sr = sizeof(sr_values) / sizeof(sr_values[0]);

    static const tflac_u8 mode_values[] = {0, 1, 2, 3, 4, 5, 255};
    size_t n_mode = sizeof(mode_values) / sizeof(mode_values[0]);

    static const tflac_u32 ch_values[] = {1, 2, 3, 6, 8, 0, 4294967295u};
    size_t n_ch = sizeof(ch_values) / sizeof(ch_values[0]);

    static const tflac_u32 bd_values[] = {8,  12, 16, 20,        24,
                                           32, 0,  1,  10,        17,
                                           64, 4294967295u};
    size_t n_bd = sizeof(bd_values) / sizeof(bd_values[0]);

    size_t i, mi, ci;

    for (i = 0; i < n_bs; i++) {
        run_case(&case_no, base_sr, base_ch, base_bd, base_mode,
                 bs_values[i]);
    }

    for (i = 0; i < n_sr; i++) {
        run_case(&case_no, sr_values[i], base_ch, base_bd, base_mode,
                 base_bs);
    }

    for (mi = 0; mi < n_mode; mi++) {
        for (ci = 0; ci < n_ch; ci++) {
            run_case(&case_no, base_sr, ch_values[ci], base_bd,
                     mode_values[mi], base_bs);
        }
    }

    for (i = 0; i < n_bd; i++) {
        run_case(&case_no, base_sr, base_ch, bd_values[i], base_mode,
                 base_bs);
    }

    {
        uint32_t state = 2463534242u;
        int trial;
        for (trial = 0; trial < 300; trial++) {
            tflac_u32 bs = bs_values[xs32(&state) % n_bs];
            tflac_u32 sr = sr_values[xs32(&state) % n_sr];
            tflac_u8 mode = mode_values[xs32(&state) % n_mode];
            tflac_u32 ch = ch_values[xs32(&state) % n_ch];
            tflac_u32 bd = bd_values[xs32(&state) % n_bd];
            run_case(&case_no, sr, ch, bd, mode, bs);
        }
    }

    return 0;
}
