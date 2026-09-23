#include <stdio.h>
#include <stdint.h>
#include <limits.h>

#include "loop.h"

static uint32_t g_rng_state = 0xC001D00Du;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

/* Deterministic pseudo-random int in [lo, hi] inclusive, lo <= hi, with
   (hi - lo) representable as int (all call sites respect this). */
static int rand_range(int lo, int hi) {
    uint32_t span = (uint32_t)(hi - lo) + 1u;
    uint32_t r = xorshift32() % span;
    return lo + (int)r;
}

static void run_case(int case_no, int max_val) {
    printf("case %d max_val=%d\n", case_no, max_val);
    loop(max_val);
    printf("case %d end\n", case_no);
}

int main(void) {
    int case_no = 0;
    int i;

    /* minimum possible value: the loop body never executes, and the
       comparison 0 <= INT_MIN is false so no arithmetic is performed */
    run_case(case_no++, INT_MIN);

    /* deeply negative, still expected to produce no output */
    run_case(case_no++, -1000000);

    /* just below the empty/non-empty boundary */
    run_case(case_no++, -1);

    /* zero: exactly one iteration, exercises the <= boundary directly */
    run_case(case_no++, 0);

    /* one: the next boundary past zero */
    run_case(case_no++, 1);

    run_case(case_no++, 2);
    run_case(case_no++, 5);
    run_case(case_no++, 10);
    run_case(case_no++, 100);

    /* moderately large, still comfortably within the output budget; the
       true maximum (INT_MAX) is deliberately not used here because the
       unit's loop increments i past max_val, which would both overflow
       signed int (i++ once i == INT_MAX) and blow far past the 256 KiB
       output budget. */
    run_case(case_no++, 1000);

    /* a spread of deterministic pseudo-random values, both negative and
       non-negative, generated with a fixed-seed xorshift generator, to
       call the unit "many times" with varied data. */
    for (i = 0; i < 10; i++) {
        int v = rand_range(-500, 500);
        run_case(case_no++, v);
    }

    return 0;
}
