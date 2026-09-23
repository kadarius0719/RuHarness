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

#define BUFN 32

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static int case_id = 0;

static void run_case(const char *label, const btac1c_s16 *ib0, const btac1c_s16 *ib1, int len) {
    btac1c_s16 buf0[BUFN];
    btac1c_s16 buf1[BUFN];
    int ret, i;
    for (i = 0; i < BUFN; i++) {
        buf0[i] = ib0[i];
        buf1[i] = ib1[i];
    }
    ret = stereo_samples(buf0, buf1, len);
    printf("case %d %s len=%d ret=%d\n", case_id++, label, len, ret);
}

int main(void) {
    uint32_t rng = 246813579u;
    btac1c_s16 zero_buf[BUFN];
    btac1c_s16 max_buf[BUFN];
    btac1c_s16 min_buf[BUFN];
    btac1c_s16 alt_buf[BUFN];
    btac1c_s16 ramp_buf[BUFN];
    btac1c_s16 rand_buf0[BUFN];
    btac1c_s16 rand_buf1[BUFN];
    int i, len, r;

    for (i = 0; i < BUFN; i++) zero_buf[i] = 0;
    for (i = 0; i < BUFN; i++) max_buf[i] = 32767;
    for (i = 0; i < BUFN; i++) min_buf[i] = -32768;
    for (i = 0; i < BUFN; i++) alt_buf[i] = (btac1c_s16)((i & 1) ? 32767 : -32768);
    for (i = 0; i < BUFN; i++) ramp_buf[i] = (btac1c_s16)((i * 2003) - 30000);

    for (len = 0; len <= 16; len++) {
        run_case("zero_vs_zero", zero_buf, zero_buf, len);
        run_case("max_vs_min", max_buf, min_buf, len);
        run_case("alt_vs_zero", alt_buf, zero_buf, len);
        run_case("ramp_vs_alt", ramp_buf, alt_buf, len);
    }

    run_case("len_17", ramp_buf, alt_buf, 17);
    run_case("len_31", ramp_buf, alt_buf, 31);
    run_case("len_32", ramp_buf, alt_buf, 32);
    run_case("len_neg1", ramp_buf, alt_buf, -1);
    run_case("len_neg16", ramp_buf, alt_buf, -16);
    run_case("len_neg17", ramp_buf, alt_buf, -17);
    run_case("len_intmax", ramp_buf, alt_buf, INT_MAX);
    run_case("len_intmin", ramp_buf, alt_buf, INT_MIN);
    run_case("len_100", ramp_buf, alt_buf, 100);
    run_case("len_1000000", ramp_buf, alt_buf, 1000000);

    for (r = 0; r < 24; r++) {
        for (i = 0; i < BUFN; i++) {
            rand_buf0[i] = (btac1c_s16)(xorshift32(&rng) & 0xFFFFu);
            rand_buf1[i] = (btac1c_s16)(xorshift32(&rng) & 0xFFFFu);
        }
        int rlen = (int)(xorshift32(&rng) % 33u) - 16;
        run_case("random", rand_buf0, rand_buf1, rlen);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
