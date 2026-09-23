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

#include "staticloop.h"

static int g_case = 0;

static void call_static_sum(int update) {
    int ret;
    g_case++;
    ret = static_sum(update);
    printf("case %d static_sum update=%d ret=%d\n", g_case, update, ret);
}

static void call_driver(int stride) {
    g_case++;
    printf("case %d driver stride=%d begin\n", g_case, stride);
    driver(stride);
    printf("case %d driver stride=%d end\n", g_case, stride);
}

int main(void) {
    call_static_sum(0);
    call_static_sum(1);
    call_static_sum(-1);
    call_static_sum(5);
    call_static_sum(-5);

    call_driver(0);
    call_driver(1);
    call_driver(-1);
    call_driver(1000);
    call_driver(-1000);
    call_driver(40000000);
    call_driver(-40000000);

    call_static_sum(INT_MAX);
    call_static_sum(-INT_MAX);
    call_static_sum(INT_MIN);
    call_static_sum(INT_MAX);
    call_static_sum(1);

    printf("final_case_count=%d\n", g_case);

    return 0;
}
