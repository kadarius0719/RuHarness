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

static void run_case(const char *tag, cb_impairment imp, int imp_id,
                      float r0, float g0, float b0) {
    float R = r0;
    float G = g0;
    float B = b0;
    colourblind(imp, &R, &G, &B);
    printf("case %d %s imp=%d out_r=%a out_g=%a out_b=%a\n",
           case_id++, tag, imp_id, (double)R, (double)G, (double)B);
}

static float scaled(uint32_t u, float lo, float hi) {
    double frac = (double)u / 4294967295.0;
    return (float)(lo + frac * (hi - lo));
}

int main(void) {
    uint32_t rng = 0x9E3779B9u;
    cb_impairment imps[3];
    int imp_ids[3];
    int k, i;

    imps[0] = cbProtanopia;
    imp_ids[0] = 0;
    imps[1] = cbDeuteranopia;
    imp_ids[1] = 1;
    imps[2] = cbTritanopia;
    imp_ids[2] = 2;

    for (k = 0; k < 3; k++) {
        run_case("zero", imps[k], imp_ids[k], 0.0f, 0.0f, 0.0f);
        run_case("r1", imps[k], imp_ids[k], 1.0f, 0.0f, 0.0f);
        run_case("g1", imps[k], imp_ids[k], 0.0f, 1.0f, 0.0f);
        run_case("b1", imps[k], imp_ids[k], 0.0f, 0.0f, 1.0f);
        run_case("white", imps[k], imp_ids[k], 1.0f, 1.0f, 1.0f);
        run_case("neg_all", imps[k], imp_ids[k], -1.0f, -1.0f, -1.0f);
        run_case("mixed_a", imps[k], imp_ids[k], 0.25f, 0.5f, 0.75f);
        run_case("mixed_b", imps[k], imp_ids[k], 0.9f, 0.1f, 0.3f);
        run_case("mixed_c", imps[k], imp_ids[k], 0.6f, 0.9f, 0.05f);
        run_case("large_pos", imps[k], imp_ids[k], 1000.0f, 2000.0f, 3000.0f);
        run_case("large_mixed", imps[k], imp_ids[k], 1000.0f, -1000.0f, 500.0f);
        run_case("small_pos", imps[k], imp_ids[k], FLT_MIN, FLT_MIN, FLT_MIN);
        run_case("small_neg", imps[k], imp_ids[k], -FLT_MIN, -FLT_MIN, -FLT_MIN);
        run_case("tiny_frac", imps[k], imp_ids[k], 1e-6f, -1e-6f, 1e-6f);
        run_case("half", imps[k], imp_ids[k], 0.5f, 0.5f, 0.5f);
        run_case("r_only_neg", imps[k], imp_ids[k], -1.0f, 0.0f, 0.0f);
        run_case("g_only_neg", imps[k], imp_ids[k], 0.0f, -1.0f, 0.0f);
        run_case("b_only_neg", imps[k], imp_ids[k], 0.0f, 0.0f, -1.0f);
        run_case("one_two_three", imps[k], imp_ids[k], 1.0f, 2.0f, 3.0f);
        run_case("three_two_one", imps[k], imp_ids[k], 3.0f, 2.0f, 1.0f);
    }

    for (k = 0; k < 3; k++) {
        for (i = 0; i < 60; i++) {
            float r0 = scaled(xorshift32(&rng), -8.0f, 8.0f);
            float g0 = scaled(xorshift32(&rng), -8.0f, 8.0f);
            float b0 = scaled(xorshift32(&rng), -8.0f, 8.0f);
            run_case("random", imps[k], imp_ids[k], r0, g0, b0);
        }
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
