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

#define ZLEN 899
#define PCMLEN 256

static uint32_t xs32(uint32_t *state) {
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

static float rand_float(uint32_t *state, float lo, float hi) {
    uint32_t r = xs32(state);
    float t = (float)r / (float)UINT32_MAX;
    return lo + t * (hi - lo);
}

static void run_case(int *case_no, mp3d_sample_t *pcm, int nch,
                      const float *z) {
    memset(pcm, 0, PCMLEN * sizeof(mp3d_sample_t));
    synth_pair(pcm, nch, z);
    printf("case %d nch=%d pcm0=%" PRId16 " pcmN=%" PRId16 "\n", *case_no,
           nch, (int16_t)pcm[0], (int16_t)pcm[16 * nch]);
    (*case_no)++;
}

int main(void) {
    float z[ZLEN];
    mp3d_sample_t pcm[PCMLEN];
    int case_no = 0;
    int nch_values[] = {0, 1, 2, 6, 8};
    size_t n_nch = sizeof(nch_values) / sizeof(nch_values[0]);
    size_t k;
    size_t ni;

    for (k = 0; k < ZLEN; k++)
        z[k] = 0.0f;
    for (ni = 0; ni < n_nch; ni++)
        run_case(&case_no, pcm, nch_values[ni], z);

    for (k = 0; k < ZLEN; k++)
        z[k] = 1.0f;
    for (ni = 0; ni < n_nch; ni++)
        run_case(&case_no, pcm, nch_values[ni], z);

    for (k = 0; k < ZLEN; k++)
        z[k] = -1.0f;
    for (ni = 0; ni < n_nch; ni++)
        run_case(&case_no, pcm, nch_values[ni], z);

    for (k = 0; k < ZLEN; k++)
        z[k] = (k % 2 == 0) ? 1.0f : -1.0f;
    for (ni = 0; ni < n_nch; ni++)
        run_case(&case_no, pcm, nch_values[ni], z);

    {
        size_t idxs[] = {0,          64,         2 * 64,     3 * 64,
                          4 * 64,     5 * 64,     6 * 64,     7 * 64,
                          8 * 64,     9 * 64,     10 * 64,    11 * 64,
                          12 * 64,    13 * 64,    14 * 64,    2,
                          2 + 2 * 64, 2 + 4 * 64, 2 + 6 * 64, 2 + 8 * 64,
                          2 + 10 * 64, 2 + 12 * 64, 2 + 14 * 64};
        size_t n_idx = sizeof(idxs) / sizeof(idxs[0]);
        size_t ii;
        for (ii = 0; ii < n_idx; ii++) {
            for (k = 0; k < ZLEN; k++)
                z[k] = 0.0f;
            z[idxs[ii]] = 500.0f;
            run_case(&case_no, pcm, 2, z);

            for (k = 0; k < ZLEN; k++)
                z[k] = 0.0f;
            z[idxs[ii]] = -500.0f;
            run_case(&case_no, pcm, 2, z);
        }
    }

    {
        uint32_t state = 2463534242u;
        int i;
        for (i = 0; i < 200; i++) {
            for (k = 0; k < ZLEN; k++)
                z[k] = rand_float(&state, -2.0f, 2.0f);
            run_case(&case_no, pcm, nch_values[i % (int)n_nch], z);
        }
        for (i = 0; i < 100; i++) {
            for (k = 0; k < ZLEN; k++)
                z[k] = rand_float(&state, -50.0f, 50.0f);
            run_case(&case_no, pcm, nch_values[i % (int)n_nch], z);
        }
    }

    return 0;
}
