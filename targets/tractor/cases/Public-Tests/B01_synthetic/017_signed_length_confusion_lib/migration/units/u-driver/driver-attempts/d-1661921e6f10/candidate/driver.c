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

void printLine(const char *line);

int main(void)
{
    int case_no = 1;
    int d;

    printf("case %d: direct printLine\n", case_no++);
    printLine("hello");
    printLine("");
    printLine(NULL);

    /*
     * The surviving mutants are the "100" that sizes `dest`
     * (`char dest[100] = "";`) and the "100"/"1" inside
     * `source[100-1] = '\0';`. Both only matter for calls that take
     * the `data < 100` branch, and only become observable once `data`
     * gets close to the buffer's real capacity. Sweep every `data`
     * value from 0 through 99 (covering every boundary a mutated
     * constant could shift to), then re-probe the values right at and
     * just below the boundary (99, 98, 97, ...) repeatedly and in a
     * different order than the forward sweep, so that neither the
     * exact call sequence nor a single isolated sample is relied on
     * to expose a shifted boundary.
     */
    for (d = 0; d < 100; d++) {
        printf("case %d: driver(%d)\n", case_no++, d);
        driver(d);
    }

    printf("case %d: driver(100)\n", case_no++);
    driver(100);

    printf("case %d: driver(101)\n", case_no++);
    driver(101);

    printf("case %d: driver(1000)\n", case_no++);
    driver(1000);

    printf("case %d: driver(INT_MAX)\n", case_no++);
    driver(INT_MAX);

    printf("case %d: driver(0) again\n", case_no++);
    driver(0);

    printf("case %d: driver(99) again\n", case_no++);
    driver(99);

    /* Reverse-order boundary re-sweep: 99 down to 90, each probed
     * three times in a row, interleaved with a small `data` value so
     * that any state a mutant might leave behind on the stack from a
     * near-boundary call cannot masquerade as the untouched initial
     * state on the next near-boundary call. */
    {
        int b;
        for (b = 99; b >= 90; b--) {
            printf("case %d: driver(%d) rsweep#1\n", case_no++, b);
            driver(b);
            printf("case %d: driver(%d) rsweep#2\n", case_no++, b);
            driver(b);
            printf("case %d: driver(1) probe\n", case_no++);
            driver(1);
            printf("case %d: driver(%d) rsweep#3\n", case_no++, b);
            driver(b);
        }
    }

    /* Interleave a large (false-branch) call between consecutive
     * near-boundary (true-branch) calls, so any leftover state from
     * the false branch cannot be confused with leftover state from a
     * true-branch call. */
    {
        static const int near[] = {99, 98, 97, 96, 95, 50, 10, 5, 1, 0};
        size_t k;
        for (k = 0; k < sizeof(near) / sizeof(near[0]); k++) {
            printf("case %d: driver(500) filler\n", case_no++);
            driver(500);
            printf("case %d: driver(%d) interleaved\n", case_no++, near[k]);
            driver(near[k]);
        }
    }

    printf("case %d: driver(99) final\n", case_no++);
    driver(99);
    printf("case %d: driver(98) final\n", case_no++);
    driver(98);

    printf("total_cases=%d\n", case_no - 1);

    return 0;
}
