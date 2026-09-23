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

static void run_case(const char *label, const int *psamp8, int idx, int pfcn,
                      const btac1c_s16 firfx[4][8]) {
    int psamp[8];
    btac1c_idxstate ridx;
    int ret, i, r;
    memset(&ridx, 0, sizeof ridx);
    for (i = 0; i < 8; i++) psamp[i] = psamp8[i];
    for (r = 0; r < 4; r++)
        for (i = 0; i < 8; i++) ridx.firfx[r][i] = firfx[r][i];
    ret = predict_sample(psamp, idx, pfcn, &ridx);
    printf("case %d %s idx=%d pfcn=%d ret=%d\n", case_id++, label, idx, pfcn, ret);
}

int main(void) {
    uint32_t rng = 999888777u;
    int p1[8] = {10, -5, 20, -15, 7, 3, -8, 12};
    int p2[8] = {0, 0, 0, 0, 0, 0, 0, 0};
    int p3[8] = {1000, -1000, 500, -500, 250, -250, 100, -100};
    btac1c_s16 fx1[4][8];
    btac1c_s16 fx2[4][8];
    int r, c, pfcn, idx_i;
    int idx_values[5] = {0, 8, -8, 1000000, -2000000000};

    for (r = 0; r < 4; r++)
        for (c = 0; c < 8; c++) {
            fx1[r][c] = (btac1c_s16)((r + 1) * 10 + c);
            fx2[r][c] = (btac1c_s16)(-((r + 1) * 5 + c));
        }

    for (pfcn = -20; pfcn <= 40; pfcn++) {
        for (idx_i = 0; idx_i < 3; idx_i++) {
            run_case("sweep_p1_fx1", p1, idx_values[idx_i], pfcn, fx1);
        }
    }

    for (pfcn = 0; pfcn <= 16; pfcn++) {
        run_case("p2_zero_fx1", p2, 8, pfcn, fx1);
        run_case("p3_extreme_fx2", p3, -8, pfcn, fx2);
    }

    run_case("idx_huge_pos", p1, idx_values[3], 7, fx1);
    run_case("idx_huge_neg", p1, idx_values[4], 9, fx2);
    run_case("pfcn_17_wrap_to0", p1, 8, 17, fx1);
    run_case("pfcn_33_wrap_default", p1, 8, 33, fx1);
    run_case("pfcn_neg17", p1, 8, -17, fx1);

    for (r = 0; r < 30; r++) {
        int rp[8];
        int i;
        for (i = 0; i < 8; i++)
            rp[i] = (int)((int32_t)(xorshift32(&rng) % 2001u) - 1000);
        int rpfcn = (int)((int32_t)(xorshift32(&rng) % 41u) - 20);
        int ridxv = (int)((int32_t)(xorshift32(&rng) % 2001u) - 1000);
        btac1c_s16 rfx[4][8];
        int rr, rc;
        for (rr = 0; rr < 4; rr++)
            for (rc = 0; rc < 8; rc++)
                rfx[rr][rc] = (btac1c_s16)((int)(xorshift32(&rng) % 2001u) - 1000);
        run_case("random", rp, ridxv, rpfcn, rfx);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
