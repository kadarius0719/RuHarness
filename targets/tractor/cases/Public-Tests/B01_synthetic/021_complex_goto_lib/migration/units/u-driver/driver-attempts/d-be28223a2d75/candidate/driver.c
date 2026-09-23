#include "driver.h"

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

/*
 * driver(x, y)'s inner "if (x < 3) goto label1;" retry loop only ever
 * breaks out through "if (y == 0) continue;" at label2. Once x has
 * counted down to <= 0 it never becomes >= 3 again (its only decrement
 * is gated by "x > 0"), so the retry keeps firing forever unless y is
 * able to reach exactly 0. y only moves by "y--", so it reaches exactly
 * 0 when it starts non-negative, but if y starts negative it only ever
 * gets more negative and can never land on exactly 0. So any call with
 * x > 0 and y < 0 walks x down until it is stuck below 3 while y keeps
 * decrementing without ever satisfying the exit check: an unconditional
 * infinite loop intrinsic to the unit, independent of the exact
 * starting magnitudes. That combination (e.g. driver(3, -5) from an
 * earlier version of this driver) is what produced unbounded output.
 * Every case below avoids x > 0 together with y < 0; every other sign
 * combination for x and y is safe: x <= 0 makes the x-decrement a
 * permanent no-op so the outer condition depends only on y, and y >= 0
 * always reaches exactly 0 and exits via "continue".
 */

static void run_case(int *case_no, int x, int y) {
    printf("case %d: driver(%d, %d)\n", *case_no, x, y);
    driver(x, y);
    (*case_no)++;
}

int main(void)
{
    int case_no = 1;

    run_case(&case_no, 0, 0);
    run_case(&case_no, 1, 0);
    run_case(&case_no, 0, 1);
    run_case(&case_no, 2, 0);
    run_case(&case_no, 0, 2);
    run_case(&case_no, 3, 0);
    run_case(&case_no, 0, 3);
    run_case(&case_no, 5, 0);
    run_case(&case_no, 0, 5);
    run_case(&case_no, 1, 1);
    run_case(&case_no, 3, 3);
    run_case(&case_no, 1, 4);
    run_case(&case_no, 4, 1);
    run_case(&case_no, 2, 2);
    run_case(&case_no, 2, 5);
    run_case(&case_no, 6, 2);
    run_case(&case_no, 2, 6);
    run_case(&case_no, 10, 10);
    run_case(&case_no, 20, 0);
    run_case(&case_no, 0, 20);
    run_case(&case_no, -1, -1);
    run_case(&case_no, -5, 3);
    run_case(&case_no, -2, 0);
    run_case(&case_no, 0, -1);
    run_case(&case_no, -3, -7);
    run_case(&case_no, 0, 0);

    return 0;
}
