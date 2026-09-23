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

static uint32_t xr32(void) {
    uint32_t x = xr_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    xr_state = x;
    return x;
}

static int g_case = 0;

static void run(unsigned char ar, unsigned char ag, unsigned char ab,
                 unsigned char br, unsigned char bg, unsigned char bb) {
    cb_rgb_255 A;
    cb_rgb_255 B;
    A.R = ar;
    A.G = ag;
    A.B = ab;
    B.R = br;
    B.G = bg;
    B.B = bb;
    float ratio = contrast_ratio(A, B);
    g_case++;
    printf("case %d A=%" PRIu32 ",%" PRIu32 ",%" PRIu32 " B=%" PRIu32 ",%" PRIu32 ",%" PRIu32 " ratio=%a\n",
           g_case, (uint32_t)ar, (uint32_t)ag, (uint32_t)ab,
           (uint32_t)br, (uint32_t)bg, (uint32_t)bb, (double)ratio);
}

int main(void) {
    xr_state = 0xDEADBEEFu;

    /* Black vs white and identical colors (ratio should be extremal or 1). */
    run(0, 0, 0, 0, 0, 0);
    run(255, 255, 255, 255, 255, 255);
    run(0, 0, 0, 255, 255, 255);
    run(255, 255, 255, 0, 0, 0);

    /* Primary and secondary swatches against black and white. */
    {
        static const unsigned char R[] = {255, 0, 0, 255, 255, 0};
        static const unsigned char G[] = {0, 255, 0, 255, 0, 255};
        static const unsigned char B[] = {0, 0, 255, 0, 255, 255};
        size_t n = sizeof(R) / sizeof(R[0]);
        for (size_t i = 0; i < n; i++) {
            run(R[i], G[i], B[i], 0, 0, 0);
            run(R[i], G[i], B[i], 255, 255, 255);
            run(0, 0, 0, R[i], G[i], B[i]);
        }
    }

    /* Boundary around the sRGB linearization threshold (0.04045 * 255 ~= 10.31). */
    for (int v = 8; v <= 13; v++) {
        run((unsigned char)v, (unsigned char)v, (unsigned char)v, 0, 0, 0);
        run((unsigned char)v, (unsigned char)v, (unsigned char)v, 255, 255, 255);
    }

    /* Isolate the threshold per channel. */
    for (int v = 9; v <= 12; v++) {
        run((unsigned char)v, 0, 0, 0, 0, 0);
        run(0, (unsigned char)v, 0, 0, 0, 0);
        run(0, 0, (unsigned char)v, 0, 0, 0);
    }

    /* Mid-range grays, both orderings so High/Low swap logic is exercised. */
    for (int v = 32; v <= 224; v += 32) {
        run((unsigned char)v, (unsigned char)v, (unsigned char)v, 128, 128, 128);
        run(128, 128, 128, (unsigned char)v, (unsigned char)v, (unsigned char)v);
    }

    /* Pseudo-random RGB pairs. */
    for (int i = 0; i < 25; i++) {
        unsigned char ar = (unsigned char)(xr32() & 0xFFu);
        unsigned char ag = (unsigned char)(xr32() & 0xFFu);
        unsigned char ab = (unsigned char)(xr32() & 0xFFu);
        unsigned char br = (unsigned char)(xr32() & 0xFFu);
        unsigned char bg = (unsigned char)(xr32() & 0xFFu);
        unsigned char bb = (unsigned char)(xr32() & 0xFFu);
        run(ar, ag, ab, br, bg, bb);
    }

    return 0;
}
