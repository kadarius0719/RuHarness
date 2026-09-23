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

#define MAX_W 16
#define MAX_H 8
#define MAX_PIX (MAX_W * MAX_H)

static uint64_t g_rng_state = 0xBF58476D1CE4E5B9ULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static uint8_t rand_u8(void) {
    return (uint8_t)(xorshift64() & 0xFFu);
}

static int g_case = 0;

static void run_case(const char *tag, int w, int h, cp_pixel_t *pix) {
    cp_image_t img;
    int i, n;

    img.w = w;
    img.h = h;
    img.pix = pix;

    premultiply(&img);

    n = w * h;
    printf("case %d tag=%s w=%d h=%d pix=[", g_case, tag, w, h);
    for (i = 0; i < n; i++) {
        if (i != 0) {
            printf(" ");
        }
        printf("%" PRIu8 ",%" PRIu8 ",%" PRIu8 ",%" PRIu8, pix[i].r,
               pix[i].g, pix[i].b, pix[i].a);
    }
    printf("]\n");
    g_case++;
}

static void fill_zero(cp_pixel_t *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i].r = 0;
        buf[i].g = 0;
        buf[i].b = 0;
        buf[i].a = 0;
    }
}

static void fill_max(cp_pixel_t *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i].r = 255;
        buf[i].g = 255;
        buf[i].b = 255;
        buf[i].a = (uint8_t)((i % 4) * 85);
    }
}

static void fill_random(cp_pixel_t *buf, int n) {
    int i;
    for (i = 0; i < n; i++) {
        buf[i].r = rand_u8();
        buf[i].g = rand_u8();
        buf[i].b = rand_u8();
        buf[i].a = rand_u8();
    }
}

int main(void) {
    static const int ws[] = { 0, 1, 0, 1, 2, 1, 2, 3, 4, 8, 16 };
    static const int hs[] = { 0, 0, 1, 1, 1, 2, 2, 2, 4, 8, 4 };
    size_t nsz = sizeof(ws) / sizeof(ws[0]);
    size_t i;
    static cp_pixel_t pix[MAX_PIX];

    for (i = 0; i < nsz; i++) {
        int w = ws[i];
        int h = hs[i];
        int n = w * h;

        fill_zero(pix, n);
        run_case("zero", w, h, pix);

        fill_max(pix, n);
        run_case("max", w, h, pix);

        fill_random(pix, n);
        run_case("random", w, h, pix);
    }

    return 0;
}
