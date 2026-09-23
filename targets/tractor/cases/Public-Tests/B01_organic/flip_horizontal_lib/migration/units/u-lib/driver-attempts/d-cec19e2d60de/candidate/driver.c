#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

static uint32_t rng_state = 0x9e3779b9u;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static void fill_image(cp_pixel_t *pix, int n) {
    int i;
    for (i = 0; i < n; ++i) {
        uint32_t r = xorshift32();
        pix[i].r = (uint8_t)(r & 0xffu);
        pix[i].g = (uint8_t)((r >> 8) & 0xffu);
        pix[i].b = (uint8_t)((r >> 16) & 0xffu);
        pix[i].a = (uint8_t)((r >> 24) & 0xffu);
    }
}

static void print_image(int case_id, int w, int h, const cp_pixel_t *pix) {
    int n = w * h;
    int i;
    printf("case %d w=%d h=%d\n", case_id, w, h);
    for (i = 0; i < n; ++i) {
        printf("case %d pix[%d] r=%" PRIu8 " g=%" PRIu8 " b=%" PRIu8
               " a=%" PRIu8 "\n",
               case_id, i, pix[i].r, pix[i].g, pix[i].b, pix[i].a);
    }
}

static void run_case(int case_id, int w, int h) {
    int n = w * h;
    size_t alloc_n = (size_t)(n > 0 ? n : 1);
    cp_pixel_t *buf = (cp_pixel_t *)malloc(alloc_n * sizeof(cp_pixel_t));
    cp_image_t img;
    if (n > 0) {
        fill_image(buf, n);
    }
    img.w = w;
    img.h = h;
    img.pix = buf;
    flip_horizontal(&img);
    print_image(case_id, w, h, buf);
    free(buf);
}

int main(void) {
    int i;
    int case_id = 1;

    run_case(case_id++, 0, 0);
    run_case(case_id++, 0, 5);
    run_case(case_id++, 5, 0);
    run_case(case_id++, 1, 1);
    run_case(case_id++, 4, 1);
    run_case(case_id++, 1, 4);
    run_case(case_id++, 3, 2);
    run_case(case_id++, 3, 3);
    run_case(case_id++, 5, 5);
    run_case(case_id++, 8, 8);
    run_case(case_id++, 1, 100);
    run_case(case_id++, 100, 1);
    run_case(case_id++, 2, 2);
    run_case(case_id++, 7, 4);
    run_case(case_id++, 6, 7);
    run_case(case_id++, 16, 16);

    for (i = 0; i < 12; ++i) {
        int w = (int)(xorshift32() % 20u) + 1;
        int h = (int)(xorshift32() % 20u) + 1;
        run_case(case_id++, w, h);
    }

    return 0;
}
