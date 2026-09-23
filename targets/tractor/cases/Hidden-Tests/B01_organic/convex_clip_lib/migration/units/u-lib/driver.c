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

#define MAXV 64

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static float rand_coord(uint32_t *s, float lo, float hi) {
    uint32_t r = xorshift32(s);
    float t = (float)(r % 1000001u) / 1000000.0f;
    return lo + t * (hi - lo);
}

static int case_id = 0;
static lm_vec2 g_poly[MAXV];
static lm_vec2 g_res[MAXV];

static void run_case(const char *label,
                      const lm_vec2 *poly, int nPoly,
                      const lm_vec2 *clip, int nClip) {
    int i, n;
    for (i = 0; i < nPoly; i++) g_poly[i] = poly[i];
    n = convex_clip(g_poly, nPoly, clip, nClip, g_res);
    printf("case %d %s nPoly=%d nClip=%d ret=%d verts", case_id++, label, nPoly, nClip, n);
    for (i = 0; i < n; i++) {
        printf(" (%a,%a)", g_res[i].x, g_res[i].y);
    }
    printf("\n");
}

int main(void) {
    uint32_t rng = 987654321u;
    int r, i;

    lm_vec2 square[4] = {{0.0f, 0.0f}, {10.0f, 0.0f}, {10.0f, 10.0f}, {0.0f, 10.0f}};
    lm_vec2 square_rev[4] = {{0.0f, 0.0f}, {0.0f, 10.0f}, {10.0f, 10.0f}, {10.0f, 0.0f}};
    lm_vec2 triangle[3] = {{0.0f, 0.0f}, {10.0f, 0.0f}, {5.0f, 10.0f}};
    lm_vec2 single_pt[1] = {{7.0f, 7.0f}};
    lm_vec2 single_pt_outside[1] = {{100.0f, 100.0f}};

    lm_vec2 clip_overlap[4] = {{5.0f, 5.0f}, {15.0f, 5.0f}, {15.0f, 15.0f}, {5.0f, 15.0f}};
    lm_vec2 clip_contain[4] = {{-5.0f, -5.0f}, {15.0f, -5.0f}, {15.0f, 15.0f}, {-5.0f, 15.0f}};
    lm_vec2 clip_outside[4] = {{20.0f, 20.0f}, {30.0f, 20.0f}, {30.0f, 30.0f}, {20.0f, 30.0f}};
    lm_vec2 clip_triangle[3] = {{0.0f, 0.0f}, {20.0f, 0.0f}, {10.0f, 20.0f}};
    lm_vec2 clip_pentagon[5] = {{5.0f, -5.0f}, {15.0f, 0.0f}, {12.0f, 12.0f}, {-2.0f, 12.0f}, {-5.0f, 0.0f}};
    lm_vec2 clip_collinear[5] = {{0.0f, 0.0f}, {5.0f, 0.0f}, {10.0f, 0.0f}, {10.0f, 10.0f}, {0.0f, 10.0f}};
    lm_vec2 clip_touch[4] = {{10.0f, 0.0f}, {20.0f, 0.0f}, {20.0f, 10.0f}, {10.0f, 10.0f}};

    run_case("square_vs_overlap", square, 4, clip_overlap, 4);
    run_case("square_vs_contain", square, 4, clip_contain, 4);
    run_case("square_vs_outside", square, 4, clip_outside, 4);
    run_case("square_vs_triangle", square, 4, clip_triangle, 3);
    run_case("square_vs_pentagon", square, 4, clip_pentagon, 5);
    run_case("square_vs_collinear_clip", square, 4, clip_collinear, 5);
    run_case("square_rev_vs_overlap", square_rev, 4, clip_overlap, 4);
    run_case("triangle_vs_overlap", triangle, 3, clip_overlap, 4);
    run_case("triangle_vs_triangle", triangle, 3, clip_triangle, 3);
    run_case("triangle_vs_pentagon", triangle, 3, clip_pentagon, 5);
    run_case("single_pt_inside", single_pt, 1, square, 4);
    run_case("single_pt_outside", single_pt_outside, 1, square, 4);
    run_case("square_vs_touch_edge", square, 4, clip_touch, 4);
    run_case("pentagon_self_vs_triangle", clip_pentagon, 5, clip_triangle, 3);
    run_case("square_vs_square_identical", square, 4, square, 4);

    for (r = 0; r < 16; r++) {
        lm_vec2 rp[5];
        int n = 3 + (int)(xorshift32(&rng) % 3u);
        for (i = 0; i < n; i++) {
            rp[i].x = rand_coord(&rng, -5.0f, 15.0f);
            rp[i].y = rand_coord(&rng, -5.0f, 15.0f);
        }
        run_case("random_poly_vs_overlap", rp, n, clip_overlap, 4);
    }

    for (r = 0; r < 8; r++) {
        lm_vec2 rc[4];
        for (i = 0; i < 4; i++) {
            rc[i].x = rand_coord(&rng, 0.0f, 20.0f);
            rc[i].y = rand_coord(&rng, 0.0f, 20.0f);
        }
        run_case("square_vs_random_clip", square, 4, rc, 4);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
