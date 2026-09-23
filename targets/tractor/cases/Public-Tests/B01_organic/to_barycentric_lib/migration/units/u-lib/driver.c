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

static lm_vec2 make(float x, float y) {
    lm_vec2 v;
    v.x = x;
    v.y = y;
    return v;
}

static void run_case(int *case_no, lm_vec2 p1, lm_vec2 p2, lm_vec2 p3,
                      lm_vec2 p) {
    lm_vec2 r = to_barycentric(p1, p2, p3, p);
    printf("case %d u=%a v=%a\n", *case_no, (double)r.x, (double)r.y);
    (*case_no)++;
}

int main(void) {
    int case_no = 0;
    lm_vec2 p1, p2, p3;

    p1 = make(0.0f, 0.0f);
    p2 = make(1.0f, 0.0f);
    p3 = make(0.0f, 1.0f);
    run_case(&case_no, p1, p2, p3, make(0.0f, 0.0f));
    run_case(&case_no, p1, p2, p3, make(1.0f, 0.0f));
    run_case(&case_no, p1, p2, p3, make(0.0f, 1.0f));
    run_case(&case_no, p1, p2, p3, make(1.0f / 3.0f, 1.0f / 3.0f));
    run_case(&case_no, p1, p2, p3, make(0.5f, 0.5f));
    run_case(&case_no, p1, p2, p3, make(2.0f, 2.0f));
    run_case(&case_no, p1, p2, p3, make(-1.0f, -1.0f));
    run_case(&case_no, p1, p2, p3, make(0.25f, 0.25f));
    run_case(&case_no, p1, p2, p3, make(10.0f, -10.0f));

    p1 = make(-5.0f, -5.0f);
    p2 = make(5.0f, -5.0f);
    p3 = make(0.0f, 5.0f);
    run_case(&case_no, p1, p2, p3, make(0.0f, 0.0f));
    run_case(&case_no, p1, p2, p3, make(-5.0f, -5.0f));
    run_case(&case_no, p1, p2, p3, make(5.0f, -5.0f));
    run_case(&case_no, p1, p2, p3, make(0.0f, 5.0f));
    run_case(&case_no, p1, p2, p3, make(100.0f, 100.0f));

    p1 = make(0.0f, 0.0f);
    p2 = make(100.0f, 0.0f);
    p3 = make(0.0f, 100.0f);
    run_case(&case_no, p1, p2, p3, make(33.0f, 33.0f));
    run_case(&case_no, p1, p2, p3, make(0.001f, 0.001f));

    p1 = make(0.0f, 0.0f);
    p2 = make(0.001f, 0.0f);
    p3 = make(0.0f, 0.001f);
    run_case(&case_no, p1, p2, p3, make(0.0003f, 0.0003f));
    run_case(&case_no, p1, p2, p3, make(0.0005f, 0.0005f));

    p1 = make(2.0f, 1.0f);
    p2 = make(1.0f, 2.0f);
    p3 = make(-1.0f, -1.0f);
    run_case(&case_no, p1, p2, p3, make(0.0f, 0.0f));
    run_case(&case_no, p1, p2, p3, make(1.0f, 1.0f));
    run_case(&case_no, p1, p2, p3, make(-2.0f, 3.0f));

    p1 = make(-1.0f, 0.0f);
    p2 = make(1.0f, 0.0f);
    p3 = make(0.0f, 3.0f);
    run_case(&case_no, p1, p2, p3, make(0.0f, 1.0f));
    run_case(&case_no, p1, p2, p3, make(-0.5f, 1.5f));

    {
        uint32_t state = 2463534242u;
        int i;
        for (i = 0; i < 300; i++) {
            lm_vec2 a, b, c, q;
            a.x = rand_float(&state, -50.0f, 50.0f);
            a.y = rand_float(&state, -50.0f, 50.0f);
            b.x = rand_float(&state, -50.0f, 50.0f);
            b.y = rand_float(&state, -50.0f, 50.0f);
            c.x = rand_float(&state, -50.0f, 50.0f);
            c.y = rand_float(&state, -50.0f, 50.0f);
            q.x = rand_float(&state, -100.0f, 100.0f);
            q.y = rand_float(&state, -100.0f, 100.0f);
            run_case(&case_no, a, b, c, q);
        }
        for (i = 0; i < 100; i++) {
            lm_vec2 a, b, c, q;
            a.x = rand_float(&state, -1.0f, 1.0f);
            a.y = rand_float(&state, -1.0f, 1.0f);
            b.x = rand_float(&state, -1.0f, 1.0f);
            b.y = rand_float(&state, -1.0f, 1.0f);
            c.x = rand_float(&state, -1.0f, 1.0f);
            c.y = rand_float(&state, -1.0f, 1.0f);
            q.x = a.x;
            q.y = a.y;
            run_case(&case_no, a, b, c, q);
        }
    }

    return 0;
}
