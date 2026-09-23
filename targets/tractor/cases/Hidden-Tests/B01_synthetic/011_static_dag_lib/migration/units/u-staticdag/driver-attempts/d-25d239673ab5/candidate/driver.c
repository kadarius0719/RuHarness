#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <limits.h>

#include "staticdag.h"

static uint32_t g_rng_state = 0x2545F491u;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

static int rand_range(int lo, int hi) {
    uint32_t span = (uint32_t)(hi - lo) + 1u;
    uint32_t r = xorshift32() % span;
    return lo + (int)r;
}

static void reset_run(int case_no, int v) {
    int r;
    printf("case %d reset new_value=%d\n", case_no, v);
    r = static_update(true, v);
    printf("case %d reset ret=%d\n", case_no, r);
}

static void read_run(int case_no) {
    int r = static_update(false, 0);
    printf("case %d read ret=%d\n", case_no, r);
}

int main(void) {
    int case_no = 0;
    int i;

    /* ---- static_update: direct exercise, update == false must never
       change state and must simply echo the current run value ---- */
    {
        int r = static_update(false, 999999);
        printf("case %d su_false new_value=%d ret=%d\n", case_no, 999999, r);
        case_no++;
    }

    {
        int fixed[] = { 0, 1, -1, 2, -2, INT_MAX, INT_MIN, INT_MAX - 1, INT_MIN + 1 };
        size_t n = sizeof(fixed) / sizeof(fixed[0]);
        size_t k;
        for (k = 0; k < n; k++) {
            int r1, r2;
            printf("case %d su_true new_value=%d\n", case_no, fixed[k]);
            r1 = static_update(true, fixed[k]);
            printf("case %d ret=%d\n", case_no, r1);
            r2 = static_update(false, 0);
            printf("case %d readback=%d\n", case_no, r2);
            case_no++;
        }
    }

    /* pseudo-random new_value across the full 32-bit domain: static_update
       only ever assigns or reads a plain int, so no arithmetic occurs and
       no value can overflow here. */
    for (i = 0; i < 20; i++) {
        int v = (int)xorshift32();
        int r1, r2;
        printf("case %d su_true_rand new_value=%d\n", case_no, v);
        r1 = static_update(true, v);
        printf("case %d ret=%d\n", case_no, r1);
        r2 = static_update(false, 0);
        printf("case %d readback=%d\n", case_no, r2);
        case_no++;
    }

    /* ---- path_add: direct exercise, each from a controlled baseline so
       the internal addition can never overflow signed int ---- */
    reset_run(case_no, 0); case_no++;
    printf("case %d path_add update=%d\n", case_no, 0);
    path_add(0);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_add update=%d\n", case_no, 1);
    path_add(1);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_add update=%d\n", case_no, -1);
    path_add(-1);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_add update=%d\n", case_no, INT_MAX);
    path_add(INT_MAX);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_add update=%d\n", case_no, INT_MIN);
    path_add(INT_MIN);
    read_run(case_no); case_no++;

    reset_run(case_no, INT_MAX); case_no++;
    printf("case %d path_add update=%d\n", case_no, 0);
    path_add(0);
    read_run(case_no); case_no++;

    reset_run(case_no, INT_MIN); case_no++;
    printf("case %d path_add update=%d\n", case_no, 0);
    path_add(0);
    read_run(case_no); case_no++;

    reset_run(case_no, 12345); case_no++;
    printf("case %d path_add update=%d\n", case_no, -6789);
    path_add(-6789);
    read_run(case_no); case_no++;

    /* accumulate several small pseudo-random deltas without resetting in
       between, to also exercise state carried across repeated calls;
       bounded so the running total can never overflow signed int */
    reset_run(case_no, 0); case_no++;
    for (i = 0; i < 14; i++) {
        int v = rand_range(-1000, 1000);
        printf("case %d path_add update=%d\n", case_no, v);
        path_add(v);
        read_run(case_no); case_no++;
    }

    /* ---- path_subtract: direct exercise ---- */
    reset_run(case_no, 0); case_no++;
    printf("case %d path_subtract update=%d\n", case_no, 0);
    path_subtract(0);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_subtract update=%d\n", case_no, 1);
    path_subtract(1);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_subtract update=%d\n", case_no, -1);
    path_subtract(-1);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_subtract update=%d\n", case_no, INT_MAX);
    path_subtract(INT_MAX);
    read_run(case_no); case_no++;

    /* run == -1 then subtract INT_MIN: -1 - INT_MIN == INT_MAX exactly,
       the one baseline that makes this particular combination overflow
       free (0 - INT_MIN would overflow and is deliberately avoided) */
    reset_run(case_no, -1); case_no++;
    printf("case %d path_subtract update=%d\n", case_no, INT_MIN);
    path_subtract(INT_MIN);
    read_run(case_no); case_no++;

    reset_run(case_no, INT_MAX); case_no++;
    printf("case %d path_subtract update=%d\n", case_no, 0);
    path_subtract(0);
    read_run(case_no); case_no++;

    reset_run(case_no, INT_MIN); case_no++;
    printf("case %d path_subtract update=%d\n", case_no, 0);
    path_subtract(0);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    for (i = 0; i < 14; i++) {
        int v = rand_range(-1000, 1000);
        printf("case %d path_subtract update=%d\n", case_no, v);
        path_subtract(v);
        read_run(case_no); case_no++;
    }

    /* ---- path_mult: direct exercise ---- */
    reset_run(case_no, 0); case_no++;
    printf("case %d path_mult update=%d\n", case_no, 0);
    path_mult(0);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_mult update=%d\n", case_no, 5);
    path_mult(5);
    read_run(case_no); case_no++;

    reset_run(case_no, 1); case_no++;
    printf("case %d path_mult update=%d\n", case_no, 0);
    path_mult(0);
    read_run(case_no); case_no++;

    reset_run(case_no, 1); case_no++;
    printf("case %d path_mult update=%d\n", case_no, 1);
    path_mult(1);
    read_run(case_no); case_no++;

    reset_run(case_no, 1); case_no++;
    printf("case %d path_mult update=%d\n", case_no, -1);
    path_mult(-1);
    read_run(case_no); case_no++;

    reset_run(case_no, INT_MAX); case_no++;
    printf("case %d path_mult update=%d\n", case_no, 1);
    path_mult(1);
    read_run(case_no); case_no++;

    reset_run(case_no, INT_MIN); case_no++;
    printf("case %d path_mult update=%d\n", case_no, 1);
    path_mult(1);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_mult update=%d\n", case_no, INT_MAX);
    path_mult(INT_MAX);
    read_run(case_no); case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d path_mult update=%d\n", case_no, INT_MIN);
    path_mult(INT_MIN);
    read_run(case_no); case_no++;

    reset_run(case_no, 100); case_no++;
    printf("case %d path_mult update=%d\n", case_no, -1);
    path_mult(-1);
    read_run(case_no); case_no++;

    reset_run(case_no, -100); case_no++;
    printf("case %d path_mult update=%d\n", case_no, -1);
    path_mult(-1);
    read_run(case_no); case_no++;

    for (i = 0; i < 9; i++) {
        int base = rand_range(-1000, 1000);
        int mult = rand_range(-5, 5);
        reset_run(case_no, base); case_no++;
        printf("case %d path_mult update=%d\n", case_no, mult);
        path_mult(mult);
        read_run(case_no); case_no++;
    }

    /* ---- driver: direct exercise ---- */
    printf("case %d driver val=%d iterations=%d\n", case_no, 0, 0);
    driver(0, 0);
    printf("case %d driver_end\n", case_no);
    case_no++;

    printf("case %d driver val=%d iterations=%d\n", case_no, 1, 0);
    driver(1, 0);
    printf("case %d driver_end\n", case_no);
    case_no++;

    printf("case %d driver val=%d iterations=%d\n", case_no, -1, 0);
    driver(-1, 0);
    printf("case %d driver_end\n", case_no);
    case_no++;

    printf("case %d driver val=%d iterations=%d\n", case_no, INT_MAX, 0);
    driver(INT_MAX, 0);
    printf("case %d driver_end\n", case_no);
    case_no++;

    printf("case %d driver val=%d iterations=%d\n", case_no, INT_MIN, 0);
    driver(INT_MIN, 0);
    printf("case %d driver_end\n", case_no);
    case_no++;

    printf("case %d driver val=%d iterations=%d\n", case_no, 5, -1);
    driver(5, -1);
    printf("case %d driver_end\n", case_no);
    case_no++;

    printf("case %d driver val=%d iterations=%d\n", case_no, INT_MIN, -100);
    driver(INT_MIN, -100);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 0, 1);
    driver(0, 1);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 0, 25);
    driver(0, 25);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 1, 1);
    driver(1, 1);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 1, 30);
    driver(1, 30);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, -1, 1);
    driver(-1, 1);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, -1, 30);
    driver(-1, 30);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 2, 15);
    driver(2, 15);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, -2, 15);
    driver(-2, 15);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 3, 8);
    driver(3, 8);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, -3, 8);
    driver(-3, 8);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 10, 6);
    driver(10, 6);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, -10, 6);
    driver(-10, 6);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, 40000, 1);
    driver(40000, 1);
    printf("case %d driver_end\n", case_no);
    case_no++;

    reset_run(case_no, 0); case_no++;
    printf("case %d driver val=%d iterations=%d\n", case_no, -40000, 1);
    driver(-40000, 1);
    printf("case %d driver_end\n", case_no);
    case_no++;

    return 0;
}
