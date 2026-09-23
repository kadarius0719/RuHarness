#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <math.h>

#include "pow.h"

/* my_pow has external linkage in the unit but is not declared in pow.h
   (only feel_the_power is); the ABI contract gives its exact C
   signature, so we declare it here ourselves. This is a plain
   declaration, never a definition, macro, or address-of use of the
   unit's symbol. */
double my_pow(double base, double exponent);

static uint32_t g_rng_state = 0xA5A5A5A5u;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

/* deterministic pseudo-random double in [lo, hi] */
static double rand_double(double lo, double hi) {
    uint32_t r = xorshift32();
    double frac = (double)r / 4294967295.0;
    return lo + frac * (hi - lo);
}

static void run_case(int case_no, double base, double exponent) {
    double r1, r2;
    printf("case %d base=%a exponent=%a\n", case_no, base, exponent);
    r1 = my_pow(base, exponent);
    printf("case %d my_pow ret=%a\n", case_no, r1);
    r2 = feel_the_power(base, exponent);
    printf("case %d feel_the_power ret=%a\n", case_no, r2);
}

int main(void) {
    int case_no = 0;
    int i;
    size_t k, n;

    const double CASES[][2] = {
        {  0.0,      0.0    },   /* 0^0 == 1, no error                       */
        {  0.0,      1.0    },   /* 0^1 == 0                                 */
        {  0.0,      2.0    },   /* 0^2 == 0                                 */
        {  0.0,     -1.0    },   /* pole -> +inf -> range error              */
        {  0.0,     -2.0    },   /* pole -> +inf -> range error              */
        { -0.0,      0.0    },   /* (-0)^0 == 1, no error                    */
        { -0.0,      3.0    },   /* odd exponent preserves sign -> -0        */
        { -0.0,      2.0    },   /* even exponent -> +0                      */
        { -0.0,     -1.0    },   /* odd negative -> -inf -> range error      */
        {  1.0,      0.0    },   /* 1^0 == 1                                 */
        {  1.0, 1000000.0   },   /* 1^big == 1, no error                     */
        {  1.0, -1000000.0  },   /* 1^-big == 1, no error                    */
        { -1.0,      2.0    },   /* even integer exponent -> 1               */
        { -1.0,      3.0    },   /* odd integer exponent -> -1               */
        { -1.0,      0.5    },   /* negative base, fractional -> NaN         */
        { -1.0,     -0.5    },   /* negative base, fractional -> NaN         */
        { -2.0,      3.0    },   /* -8                                       */
        { -2.0,     -3.0    },   /* -0.125                                   */
        { -2.0,      2.5    },   /* NaN -> domain error                      */
        {  2.0,     10.0    },   /* 1024                                     */
        {  2.0,    -10.0    },   /* 0.0009765625                             */
        {  2.0,      0.5    },   /* sqrt(2)                                  */
        { 10.0,    400.0    },   /* overflow -> +inf -> range error          */
        { 10.0,   -400.0    },   /* underflow -> 0, no error                 */
        {-10.0,    400.0    },   /* even huge exponent -> +inf -> range err  */
        {-10.0,    401.0    },   /* odd huge exponent -> -inf -> range err   */
        {  1e200,    2.0    },   /* overflow -> +inf -> range error          */
        {  3.0,      3.0    },   /* 27                                       */
        {  5.5,      2.0    },   /* 30.25                                    */
        {  2.5,     -2.0    },   /* 0.16                                     */
        {100.0,      0.5    },   /* 10                                       */
        {  7.0,      0.0    },   /* 1                                        */
        {  0.5,     10.0    },   /* 0.0009765625                             */
    };

    /* fixed edge-case vectors */
    n = sizeof(CASES) / sizeof(CASES[0]);
    for (k = 0; k < n; k++) {
        run_case(case_no, CASES[k][0], CASES[k][1]);
        case_no++;
    }

    /* explicit NaN / infinity inputs: all well defined per the C standard's
       special-case rules for pow(), never undefined behavior */
    run_case(case_no++, NAN, 2.0);          /* NaN base, nonzero exp -> NaN  */
    run_case(case_no++, 5.0, NAN);          /* NaN exponent -> NaN           */
    run_case(case_no++, 1.0, NAN);          /* base == 1 -> 1, no error      */
    run_case(case_no++, NAN, 0.0);          /* any base, exp 0 -> 1          */
    run_case(case_no++, 0.0, NAN);          /* NaN exponent -> NaN           */
    run_case(case_no++, INFINITY, 0.0);     /* inf^0 -> 1, no error          */
    run_case(case_no++, INFINITY, 2.0);     /* inf -> range error            */
    run_case(case_no++, INFINITY, -1.0);    /* inf^-1 -> 0, no error         */
    run_case(case_no++, -INFINITY, 3.0);    /* -inf odd -> -inf, range error */
    run_case(case_no++, -INFINITY, 2.0);    /* -inf even -> +inf, range err  */
    run_case(case_no++, -INFINITY, -1.0);   /* -inf^-1 -> -0, no error       */

    /* pseudo-random pairs across a broad, mixed-sign range, deliberately
       including negative bases with fractional exponents (NaN / domain
       error path) as well as ordinary successful evaluations */
    for (i = 0; i < 15; i++) {
        double b = rand_double(-50.0, 50.0);
        double e = rand_double(-8.0, 8.0);
        run_case(case_no, b, e);
        case_no++;
    }

    /* pseudo-random pairs restricted to a positive base, to bias toward
       the ordinary (no-error) fractional-power path */
    for (i = 0; i < 10; i++) {
        double b = rand_double(0.0001, 100.0);
        double e = rand_double(-8.0, 8.0);
        run_case(case_no, b, e);
        case_no++;
    }

    /* pseudo-random pairs chosen to be likely to overflow, exercising the
       range-error branch with varied magnitudes */
    for (i = 0; i < 5; i++) {
        double b = rand_double(50.0, 500.0);
        double e = rand_double(50.0, 300.0);
        run_case(case_no, b, e);
        case_no++;
    }

    return 0;
}
