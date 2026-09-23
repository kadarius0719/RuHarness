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

/* The unit's .c file defines these types and functions with external
   linkage but they are not declared in lib.h. Reconstruct matching
   declarations here per the ABI CONTRACT so they can be called. */
typedef struct c2v {
    float x;
    float y;
} c2v;

typedef struct c2Circle {
    c2v p;
    float r;
} c2Circle;

typedef struct c2AABB {
    c2v min;
    c2v max;
} c2AABB;

c2v c2V(float x, float y);
c2v c2Maxv(c2v a, c2v b);
c2v c2Minv(c2v a, c2v b);
c2v c2Clampv(c2v a, c2v lo, c2v hi);
c2v c2Sub(c2v a, c2v b);
float c2Dot(c2v a, c2v b);
int c2CircletoCircle(c2Circle A, c2Circle B);
int c2CircletoAABB(c2Circle A, c2AABB B);
int c2AABBtoAABB(c2AABB A, c2AABB B);

#define NSAMP 16

static const float SAMPLE_F[NSAMP] = {
    0.0f, -0.0f, 1.0f, -1.0f, 0.5f, -0.5f, 123.456f, -123.456f,
    FLT_MAX, -FLT_MAX, FLT_MIN, -FLT_MIN, 1000000.0f, -1000000.0f, 7.25f, -7.25f
};

static uint32_t xr_state;

static uint32_t xr32(void) {
    uint32_t x = xr_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    xr_state = x;
    return x;
}

static float rand_float(void) {
    uint32_t r = xr32();
    return ((float)(r % 2000001u) - 1000000.0f) / 1000.0f;
}

static int g_case = 0;

static void print_v(const char *name, c2v v) {
    g_case++;
    printf("case %d %s x=%a y=%a\n", g_case, name, (double)v.x, (double)v.y);
}

static void print_f(const char *name, float f) {
    g_case++;
    printf("case %d %s val=%a\n", g_case, name, (double)f);
}

static void print_i(const char *name, int r) {
    g_case++;
    printf("case %d %s ret=%d\n", g_case, name, r);
}

int main(void) {
    xr_state = 0x2545F491u;

    c2v v[NSAMP];
    for (int i = 0; i < NSAMP; i++) {
        v[i] = c2V(SAMPLE_F[i], SAMPLE_F[(i + 3) % NSAMP]);
        print_v("c2V", v[i]);
    }

    for (int i = 0; i < NSAMP; i++) {
        c2v r = c2Maxv(v[i], v[(i + 1) % NSAMP]);
        print_v("c2Maxv", r);
    }

    for (int i = 0; i < NSAMP; i++) {
        c2v r = c2Minv(v[i], v[(i + 1) % NSAMP]);
        print_v("c2Minv", r);
    }

    for (int i = 0; i < NSAMP; i++) {
        c2v r = c2Sub(v[i], v[(i + 1) % NSAMP]);
        print_v("c2Sub", r);
    }

    for (int i = 0; i < NSAMP; i++) {
        float r = c2Dot(v[i], v[(i + 1) % NSAMP]);
        print_f("c2Dot", r);
    }

    for (int i = 0; i < NSAMP; i++) {
        c2v r = c2Clampv(v[i], v[(i + 1) % NSAMP], v[(i + 2) % NSAMP]);
        print_v("c2Clampv", r);
    }

    /* extra random Sub/Dot coverage */
    for (int i = 0; i < 10; i++) {
        c2v a = c2V(rand_float(), rand_float());
        c2v b = c2V(rand_float(), rand_float());
        print_v("c2V_rand_a", a);
        print_v("c2V_rand_b", b);
        c2v s = c2Sub(a, b);
        print_v("c2Sub_rand", s);
        float d = c2Dot(a, b);
        print_f("c2Dot_rand", d);
    }

    /* circles and AABBs built from the vector set (non-inverted by construction) */
    c2Circle circ[NSAMP];
    c2AABB box[NSAMP];
    for (int i = 0; i < NSAMP; i++) {
        float r = SAMPLE_F[(i + 5) % NSAMP];
        if (r < 0.0f) {
            r = -r;
        }
        circ[i].p = v[i];
        circ[i].r = r;

        box[i].min = v[i];
        box[i].max = c2Maxv(v[i], v[(i + 1) % NSAMP]);
    }

    /* one degenerate (zero-size) AABB and one intentionally inverted AABB */
    c2AABB degenerate;
    degenerate.min = c2V(2.0f, -3.0f);
    degenerate.max = c2V(2.0f, -3.0f);
    c2AABB inverted;
    inverted.min = c2V(5.0f, 5.0f);
    inverted.max = c2V(-5.0f, -5.0f);

    for (int i = 0; i < NSAMP; i++) {
        int r = c2CircletoCircle(circ[i], circ[(i + 1) % NSAMP]);
        print_i("c2CircletoCircle", r);
    }

    for (int i = 0; i < NSAMP; i++) {
        int r = c2CircletoAABB(circ[i], box[(i + 1) % NSAMP]);
        print_i("c2CircletoAABB", r);
    }

    for (int i = 0; i < NSAMP; i++) {
        int r = c2AABBtoAABB(box[i], box[(i + 1) % NSAMP]);
        print_i("c2AABBtoAABB", r);
    }

    {
        int r = c2AABBtoAABB(degenerate, box[0]);
        print_i("c2AABBtoAABB_degenerate", r);
        r = c2AABBtoAABB(inverted, box[0]);
        print_i("c2AABBtoAABB_inverted", r);
        r = c2CircletoAABB(circ[0], degenerate);
        print_i("c2CircletoAABB_degenerate", r);
    }

    /* dispatcher: exercise every valid type combination */
    for (int i = 0; i < 6; i++) {
        int r;
        r = collided(&circ[i], C2_TYPE_CIRCLE, &circ[(i + 1) % NSAMP], C2_TYPE_CIRCLE);
        print_i("collided_circle_circle", r);
        r = collided(&circ[i], C2_TYPE_CIRCLE, &box[(i + 1) % NSAMP], C2_TYPE_AABB);
        print_i("collided_circle_aabb", r);
        r = collided(&box[i], C2_TYPE_AABB, &circ[(i + 1) % NSAMP], C2_TYPE_CIRCLE);
        print_i("collided_aabb_circle", r);
        r = collided(&box[i], C2_TYPE_AABB, &box[(i + 1) % NSAMP], C2_TYPE_AABB);
        print_i("collided_aabb_aabb", r);
    }

    /* dispatcher default branches: an out-of-range C2_TYPE value on each side */
    {
        int r = collided(&circ[0], (C2_TYPE)2, &circ[1], C2_TYPE_CIRCLE);
        print_i("collided_outer_default", r);
        r = collided(&circ[0], C2_TYPE_CIRCLE, &circ[1], (C2_TYPE)3);
        print_i("collided_inner_circle_default", r);
        r = collided(&box[0], C2_TYPE_AABB, &box[1], (C2_TYPE)4);
        print_i("collided_inner_aabb_default", r);
    }

    return 0;
}
