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

#include "staticalias.h"

static int g_case = 0;

static void call_alias(int val) {
    int x = val;
    int *r;
    g_case++;
    r = static_alias(&x);
    printf("case %d static_alias val=%d x_after=%d r_deref=%d\n", g_case, val, x, *r);
}

static void call_driver(int initial_value, int iterations) {
    g_case++;
    printf("case %d driver begin initial_value=%d iterations=%d\n", g_case, initial_value, iterations);
    driver(initial_value, iterations);
    printf("case %d driver end\n", g_case);
}

int main(void) {
    call_alias(0);
    call_alias(1);
    call_alias(-1);
    call_alias(2);
    call_alias(3);
    call_alias(4);
    call_alias(-100);
    call_alias(100);
    call_alias(0);
    call_alias(108);

    call_driver(0, 0);
    call_driver(0, -1);
    call_driver(INT_MIN, 1);
    call_driver(0, 1);
    call_driver(300, 1);
    call_driver(1000, 2);
    call_driver(5000, 5);
    call_driver(1000000, 10);

    call_alias(INT_MAX - 577798144);
    call_alias(INT_MIN);

    printf("final_case_count=%d\n", g_case);

    return 0;
}
