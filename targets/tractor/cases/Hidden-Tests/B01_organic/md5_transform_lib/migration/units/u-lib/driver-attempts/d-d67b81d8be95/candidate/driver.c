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

tflac_u32 tflac_unpack_u32le(const tflac_u8 *d);

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static int case_id = 0;

static void run_transform(const char *label, tflac_u32 a0, tflac_u32 b0,
                           tflac_u32 c0, tflac_u32 d0, const tflac_u8 *buf64) {
    tflac_md5 m;
    memset(&m, 0, sizeof m);
    m.a = a0;
    m.b = b0;
    m.c = c0;
    m.d = d0;
    memcpy(m.buffer, buf64, 64);
    md5_transform(&m);
    printf("case %d transform_%s a=%" PRIx32 " b=%" PRIx32 " c=%" PRIx32 " d=%" PRIx32 "\n",
           case_id++, label, m.a, m.b, m.c, m.d);
}

static void run_unpack(const char *label, const tflac_u8 *d4) {
    tflac_u32 v = tflac_unpack_u32le(d4);
    printf("case %d unpack_%s bytes=%02x,%02x,%02x,%02x ret=%" PRIx32 "\n",
           case_id++, label, (unsigned)d4[0], (unsigned)d4[1], (unsigned)d4[2],
           (unsigned)d4[3], v);
}

int main(void) {
    uint32_t rng = 20240517u;
    tflac_u8 buf_zero[64];
    tflac_u8 buf_max[64];
    tflac_u8 buf_ramp[64];
    tflac_u8 buf_abc[64];
    tflac_u8 buf_rand[64];
    tflac_u8 four[4];
    int i, r;

    memset(buf_zero, 0x00, sizeof buf_zero);
    memset(buf_max, 0xFF, sizeof buf_max);
    for (i = 0; i < 64; i++) buf_ramp[i] = (tflac_u8)(i * 3);

    memset(buf_abc, 0, sizeof buf_abc);
    buf_abc[0] = 0x61;
    buf_abc[1] = 0x62;
    buf_abc[2] = 0x63;
    buf_abc[3] = 0x80;
    buf_abc[56] = 0x18;

    run_transform("zero_buf_zero_iv", 0, 0, 0, 0, buf_zero);
    run_transform("zero_buf_std_iv", 0x67452301u, 0xefcdab89u, 0x98badcfeu, 0x10325476u, buf_zero);
    run_transform("max_buf_std_iv", 0x67452301u, 0xefcdab89u, 0x98badcfeu, 0x10325476u, buf_max);
    run_transform("ramp_buf_std_iv", 0x67452301u, 0xefcdab89u, 0x98badcfeu, 0x10325476u, buf_ramp);
    run_transform("abc_block_std_iv", 0x67452301u, 0xefcdab89u, 0x98badcfeu, 0x10325476u, buf_abc);
    run_transform("abc_block_zero_iv", 0, 0, 0, 0, buf_abc);
    run_transform("max_buf_max_iv", 0xFFFFFFFFu, 0xFFFFFFFFu, 0xFFFFFFFFu, 0xFFFFFFFFu, buf_max);
    run_transform("ramp_buf_mixed_iv", 0x11111111u, 0x22222222u, 0x33333333u, 0x44444444u, buf_ramp);
    run_transform("zero_buf_max_iv", 0xFFFFFFFFu, 0xFFFFFFFFu, 0xFFFFFFFFu, 0xFFFFFFFFu, buf_zero);

    for (r = 0; r < 16; r++) {
        for (i = 0; i < 64; i++) buf_rand[i] = (tflac_u8)(xorshift32(&rng) & 0xFFu);
        tflac_u32 a0 = xorshift32(&rng);
        tflac_u32 b0 = xorshift32(&rng);
        tflac_u32 c0 = xorshift32(&rng);
        tflac_u32 d0 = xorshift32(&rng);
        run_transform("random", a0, b0, c0, d0, buf_rand);
    }

    four[0] = 0; four[1] = 0; four[2] = 0; four[3] = 0;
    run_unpack("zero", four);
    four[0] = 0xFF; four[1] = 0xFF; four[2] = 0xFF; four[3] = 0xFF;
    run_unpack("max", four);
    four[0] = 0; four[1] = 1; four[2] = 2; four[3] = 3;
    run_unpack("ascending", four);
    four[0] = 3; four[1] = 2; four[2] = 1; four[3] = 0;
    run_unpack("descending", four);
    four[0] = 0x78; four[1] = 0x56; four[2] = 0x34; four[3] = 0x12;
    run_unpack("classic_le", four);
    four[0] = 0x01; four[1] = 0x00; four[2] = 0x00; four[3] = 0x80;
    run_unpack("high_bit", four);

    for (r = 0; r < 16; r++) {
        for (i = 0; i < 4; i++) four[i] = (tflac_u8)(xorshift32(&rng) & 0xFFu);
        run_unpack("random", four);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
