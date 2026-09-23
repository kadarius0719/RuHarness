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

#include "pow.h"

struct pow_case {
    double base;
    double exponent;
};

/*
 * volatile: every previous version of this driver stored the test
 * vectors in a `static const` array. Because the array elements were
 * then compile-time constants, the optimizer was able to constant-fold
 * the internal pow(base, exponent) call inside my_pow() for those
 * inputs -- which elides pow()'s errno side effect entirely. Marking
 * the case table volatile forces a genuine runtime load for every
 * base/exponent pair, so pow() actually runs at runtime and genuinely
 * sets errno, so the EDOM/ERANGE branch is genuinely taken for the
 * cases designed to trigger it.
 *
 * The surviving mutants reported are:
 *   - the two fprintf() format-string literals written to stderr
 *     (the "Domain error..." and "Range error..." text): stderr is
 *     never read back by this driver (the contract forbids the driver
 *     from touching files/streams other than stdout), so those two
 *     literals cannot be distinguished through this driver's output.
 *   - the two `return -1;` statements that end the EDOM and ERANGE
 *     branches: these ARE observable through the printed return
 *     value, so this revision widens and multiplies the set of inputs
 *     that land in each branch (varying magnitude, sign and closeness
 *     to the boundary) so that a mutated constant on either return is
 *     forced to show up as a different `ret=` bit pattern on many
 *     independent cases rather than relying on a single case per
 *     branch.
 */
static volatile struct pow_case CASES[] = {
    {0.0, 0.0},
    {0.0, 1.0},
    {0.0, 2.0},
    {0.0, 3.0},
    {0.0, -1.0},
    {0.0, -2.0},
    {0.0, -3.0},
    {0.0, -0.5},
    {0.0, -4.0},
    {0.0, -100.0},
    {-0.0, -1.0},
    {-0.0, -2.0},
    {-0.0, -3.0},
    {1.0, 0.0},
    {1.0, 1.0},
    {1.0, -1.0},
    {1.0, 1000000.0},
    {2.0, 0.0},
    {2.0, 1.0},
    {2.0, 10.0},
    {2.0, -1.0},
    {2.0, 0.5},
    {-2.0, 3.0},
    {-2.0, 4.0},
    {-2.0, -3.0},
    {-1.0, 3.0},
    {-1.0, 0.5},
    {-1.0, -0.5},
    {-1.0, 1.5},
    {-1.0, -1.5},
    {-1.0, 2.5},
    {-3.0, 0.5},
    {-3.0, -0.5},
    {-5.0, 1.5},
    {-0.5, 0.5},
    {-0.5, -0.5},
    {-100.0, 0.25},
    {-3.5, 2.5},
    {-4.0, 0.5},
    {-8.0, 0.5},
    {-8.0, -0.5},
    {-2.0, 1.0000001},
    {-2.0, -1.0000001},
    {-1e10, 0.3},
    {-1e-10, 0.3},
    {10.0, 320.0},
    {10.0, -320.0},
    {7.0, 500.0},
    {2.0, 1075.0},
    {2.0, -1075.0},
    {100.0, 400.0},
    {5.0, -400.0},
    {1e150, 10.0},
    {1e-150, -10.0},
    {1e-200, 50.0},
    {1e-10, 400.0},
    {DBL_MAX, 1.0},
    {-DBL_MAX, 1.0},
    {DBL_MIN, 1.0},
    {2.0, DBL_MAX},
    {2.0, -DBL_MAX},
    {0.0, DBL_MAX},
    {0.0, -DBL_MAX},
    {NAN, 2.0},
    {2.0, NAN},
    {INFINITY, 2.0},
    {2.0, INFINITY},
    {0.5, INFINITY}
};

static void run_case(int case_no, double base, double exponent) {
    double result;
    int saved_errno;

    result = my_pow(base, exponent);
    saved_errno = errno;

    printf("case %d base=%a exponent=%a ret=%a errno=%d\n",
           case_no, base, exponent, result, saved_errno);
}

int main(void) {
    size_t n = sizeof(CASES) / sizeof(CASES[0]);
    size_t i;

    for (i = 0; i < n; i++) {
        double b = CASES[i].base;
        double e = CASES[i].exponent;
        run_case((int)(i + 1), b, e);
    }

    printf("total_cases=%d\n", (int)n);

    return 0;
}
