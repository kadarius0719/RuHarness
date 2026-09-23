#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>
#include <math.h>

static uint32_t rng_state = 0xabcdef01u;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static float rand_unit_float(void) {
    uint32_t r = xorshift32();
    return (float)(r >> 8) / (float)(1u << 24);
}

/* gaussian_kernel's loop runs from r = -hsize to r = hsize inclusive where
   hsize = size / 2 (C truncating division). For non-negative hsize this
   writes (2*hsize + 1) elements, which is size + 1 whenever size is even
   (an off-by-one past the nominal "size" elements) and size when size is
   odd; for size in {-1, 0} it still writes exactly one element; for
   size <= -2 it writes nothing at all. This computes exactly how many
   leading elements of dest were actually written, so the driver never
   reads or prints uninitialized memory. */
static int written_count(int size) {
    int hsize = size / 2;
    if (hsize < 0) {
        return 0;
    }
    return 2 * hsize + 1;
}

static int g_case_id = 1;

static void run_case(int size, float radius) {
    int alloc_n = (size > 0) ? size + 4 : 4;
    int wc = written_count(size);
    int i;
    float *buf = (float *)malloc((size_t)alloc_n * sizeof(float));
    for (i = 0; i < alloc_n; ++i) {
        buf[i] = 0.0f;
    }
    gaussian_kernel(buf, size, radius);
    printf("case %d size=%d radius=%a wc=%d\n", g_case_id, size, radius, wc);
    for (i = 0; i < wc; ++i) {
        printf("case %d out[%d]=%a\n", g_case_id, i, (double)buf[i]);
    }
    free(buf);
    g_case_id++;
}

int main(void) {
    static const int sizes[] = {0,  1,  -1, 2,  3,   4,   5,  6,
                                 7,  8,  9,  16, 17,  32,  -2, -5, -10};
    static const float radii[] = {0.25f,   0.5f,  1.0f,  1.6f, 2.25f,
                                   3.0f,    10.0f, 100.0f, 0.0001f, -1.5f};
    size_t ns = sizeof(sizes) / sizeof(sizes[0]);
    size_t nr = sizeof(radii) / sizeof(radii[0]);
    size_t si, ri;
    int i;

    for (si = 0; si < ns; ++si) {
        for (ri = 0; ri < nr; ++ri) {
            run_case(sizes[si], radii[ri]);
        }
    }

    for (i = 0; i < 15; ++i) {
        int size = (int)(xorshift32() % 40u) + 1;
        float radius = 0.1f + rand_unit_float() * 50.0f;
        run_case(size, radius);
    }

    return 0;
}
