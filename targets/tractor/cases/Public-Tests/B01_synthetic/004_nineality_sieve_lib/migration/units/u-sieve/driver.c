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

#include "sieve.h"

static void run_case(int case_no, int val) {
    printf("case %d start val=%d\n", case_no, val);
    sieve(val);
    printf("case %d end\n", case_no);
}

int main(void) {
    static const int cases[] = {
        9,
        19,
        99,
        0,
        1,
        8,
        10,
        -1,
        -9,
        -10,
        -19,
        -100,
        -1000,
        2147483639
    };
    const size_t n = sizeof(cases) / sizeof(cases[0]);
    size_t i;

    for (i = 0; i < n; i++) {
        run_case((int)(i + 1), cases[i]);
    }

    printf("total_cases=%d\n", (int)n);

    return 0;
}
