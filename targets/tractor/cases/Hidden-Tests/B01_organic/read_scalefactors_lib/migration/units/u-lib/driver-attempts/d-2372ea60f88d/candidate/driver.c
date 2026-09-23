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

#define MAXBANDS 20
#define BUFBYTES 64

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static int case_id = 0;

static void run_case(const char *label, const uint8_t *buf, int limit_bits,
                      const uint8_t *pba_vals, const uint8_t *scfcod_vals, int bands) {
    bs_t bs;
    uint8_t pba[MAXBANDS];
    uint8_t scfcod[MAXBANDS];
    float scf[MAXBANDS * 3];
    int i;

    bs.buf = buf;
    bs.pos = 0;
    bs.limit = limit_bits;
    for (i = 0; i < bands; i++) {
        pba[i] = pba_vals[i];
        scfcod[i] = scfcod_vals[i];
    }
    for (i = 0; i < bands * 3; i++) scf[i] = 0.0f;

    read_scalefactors(&bs, pba, scfcod, bands, scf);

    printf("case %d %s bands=%d final_pos=%d scf=", case_id++, label, bands, bs.pos);
    for (i = 0; i < bands * 3; i++) {
        printf("%a ", (double)scf[i]);
    }
    printf("\n");
}

int main(void) {
    uint32_t rng = 555444333u;
    uint8_t buf_zero[BUFBYTES];
    uint8_t buf_max[BUFBYTES];
    uint8_t buf_ramp[BUFBYTES];
    uint8_t buf_rand[BUFBYTES];
    uint8_t pba_all0[MAXBANDS];
    uint8_t pba_mid[MAXBANDS];
    uint8_t pba_lo[MAXBANDS];
    uint8_t pba_hi[MAXBANDS];
    uint8_t pba_mix[MAXBANDS];
    uint8_t scfcod_zero[MAXBANDS];
    uint8_t scfcod_small[MAXBANDS];
    uint8_t scfcod_31[MAXBANDS];
    int i, r;

    memset(buf_zero, 0x00, sizeof buf_zero);
    memset(buf_max, 0xFF, sizeof buf_max);
    for (i = 0; i < BUFBYTES; i++) buf_ramp[i] = (uint8_t)(i * 5 + 1);

    for (i = 0; i < MAXBANDS; i++) {
        pba_all0[i] = 0;
        pba_mid[i] = 10;
        pba_lo[i] = 2;
        pba_hi[i] = 19;
        pba_mix[i] = (uint8_t)((i % 2 == 0) ? 0 : (2 + (i % 18)));
        scfcod_zero[i] = 0;
        scfcod_small[i] = (uint8_t)(i % 4);
        scfcod_31[i] = 31;
    }

    run_case("zero_ba_zero_buf", buf_zero, 4096, pba_all0, scfcod_zero, 5);
    run_case("zero_bands", buf_zero, 4096, pba_all0, scfcod_zero, 0);
    run_case("mid_ba_zero_buf", buf_zero, 4096, pba_mid, scfcod_zero, 5);
    run_case("mid_ba_max_buf", buf_max, 4096, pba_mid, scfcod_zero, 5);
    run_case("mid_ba_ramp_buf", buf_ramp, 4096, pba_mid, scfcod_small, 8);
    run_case("lo_ba_ramp_buf", buf_ramp, 4096, pba_lo, scfcod_small, 8);
    run_case("hi_ba_ramp_buf", buf_ramp, 4096, pba_hi, scfcod_small, 8);
    run_case("mixed_ba_ramp_buf", buf_ramp, 4096, pba_mix, scfcod_small, 16);
    run_case("mixed_ba_scfcod31", buf_ramp, 4096, pba_mix, scfcod_31, 16);
    run_case("truncated_limit_small", buf_ramp, 10, pba_mid, scfcod_zero, 10);
    run_case("truncated_limit_zero", buf_ramp, 0, pba_mid, scfcod_zero, 5);
    run_case("truncated_limit_mid", buf_ramp, 60, pba_hi, scfcod_small, 16);
    run_case("single_band_lo", buf_max, 4096, pba_lo, scfcod_zero, 1);
    run_case("single_band_hi", buf_max, 4096, pba_hi, scfcod_zero, 1);
    run_case("many_bands_mid", buf_ramp, 4096, pba_mid, scfcod_small, 20);

    for (r = 0; r < 16; r++) {
        uint8_t rpba[MAXBANDS];
        uint8_t rscfcod[MAXBANDS];
        int n = 1 + (int)(xorshift32(&rng) % (uint32_t)(MAXBANDS - 1));
        for (i = 0; i < BUFBYTES; i++) buf_rand[i] = (uint8_t)(xorshift32(&rng) & 0xFFu);
        for (i = 0; i < n; i++) {
            uint32_t pick = xorshift32(&rng) % 19u;
            rpba[i] = (uint8_t)(pick == 0 ? 0 : (pick + 1));
            rscfcod[i] = (uint8_t)(xorshift32(&rng) % 32u);
        }
        int lim = (int)(xorshift32(&rng) % 600u);
        run_case("random", buf_rand, lim, rpba, rscfcod, n);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
