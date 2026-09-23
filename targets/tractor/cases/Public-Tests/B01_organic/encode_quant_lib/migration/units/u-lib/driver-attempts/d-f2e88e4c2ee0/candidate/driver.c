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

static int32_t bounded(uint32_t r, int32_t lo, int32_t hi) {
    uint32_t span = (uint32_t)(hi - lo) + 1u;
    return lo + (int32_t)(r % span);
}

static void run(int uni, int step, int pred, int tgt, int tgt2, int lsbit) {
    int r = encode_quant(uni, step, pred, tgt, tgt2, lsbit);
    g_case++;
    printf("case %d uni=%d step=%d pred=%d tgt=%d tgt2=%d lsbit=%d ret=%d\n",
           g_case, uni, step, pred, tgt, tgt2, lsbit, r);
}

int main(void) {
    xr_state = 0x77665544u;

    /* Sweep A: uni across several group-boundary crossings, all lsbit modes. */
    static const int UNI_VALS[] = {
        -4, -1, 0, 1, 2, 6, 7, 8, 9, 14, 15, 16, 17, 23, 24, 31, 32
    };
    size_t nuni = sizeof(UNI_VALS) / sizeof(UNI_VALS[0]);
    for (size_t i = 0; i < nuni; i++) {
        for (int lsbit = 0; lsbit <= 5; lsbit++) {
            run(UNI_VALS[i], 137, 53, 91, -64, lsbit);
        }
    }

    /* Sweep B: step magnitude, including zero and large values. */
    static const int STEP_VALS[] = {0, 1, 5, 100, 1000, 100000};
    static const int UNI_FEW[] = {3, 8, -2};
    for (size_t s = 0; s < sizeof(STEP_VALS) / sizeof(STEP_VALS[0]); s++) {
        for (size_t u = 0; u < sizeof(UNI_FEW) / sizeof(UNI_FEW[0]); u++) {
            run(UNI_FEW[u], STEP_VALS[s], 10, -10, 5, 0);
        }
    }

    /* Sweep C: pred/tgt/tgt2 extremes, both signs. */
    static const int PRED_VALS[] = {0, 1000, -1000, 100000, -100000};
    static const int TGT_VALS[] = {0, -1000, 1000, -100000, 0};
    static const int TGT2_VALS[] = {0, 1000, -1000, 0, 100000};
    for (size_t i = 0; i < 5; i++) {
        run(5, 50, PRED_VALS[i], TGT_VALS[i], TGT2_VALS[i], 1);
    }

    /* Pseudo-random combinations within a wide, overflow-safe envelope. */
    for (int i = 0; i < 40; i++) {
        int uni = bounded(xr32(), -50, 80);
        int step = bounded(xr32(), 0, 100000);
        int pred = bounded(xr32(), -100000, 100000);
        int tgt = bounded(xr32(), -100000, 100000);
        int tgt2 = bounded(xr32(), -100000, 100000);
        int lsbit = bounded(xr32(), 0, 5);
        run(uni, step, pred, tgt, tgt2, lsbit);
    }

    return 0;
}
