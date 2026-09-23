#include "driver.h"

#include <stdio.h>
#include <limits.h>

extern void fma_array(int *out, const int *mul1, const int *mul2, const int *add, int len);

static void call_fma(int case_num, int *out, const int *mul1, const int *mul2, const int *add, int len) {
    printf("case %d fma_array(len=%d)\n", case_num, len);
    fma_array(out, mul1, mul2, add, len);
    for (int i = 0; i < len; i++) {
        printf("case %d out[%d]=%d\n", case_num, i, out[i]);
    }
}

static void call_driver(int case_num, const int *data, int len) {
    printf("case %d driver(len=%d)\n", case_num, len);
    driver(data, len);
}

int main(void) {
    int case_num = 1;
    int out_buf[64];

    /* Direct fma_array calls with out distinct from mul1/mul2/add.
       Every row is chosen so mul1[i]*mul2[i]+add[i] cannot overflow
       int, including rows that land exactly on INT_MAX/INT_MIN, rows
       that multiply by 0 at the extremes, and rows with large-magnitude
       operands on both the positive and negative side. */
    {
        const int mul1[] = {0, 1, -1, -1, 5, INT_MAX, INT_MIN, INT_MAX, INT_MIN, INT_MAX, INT_MIN, 46340, -46340, 2, -2, 3, -3, 100, -100};
        const int mul2[] = {0, 1, 1, -1, -3, 1, 1, 0, 0, 1, 1, 46340, 46340, 1000000000, 1000000000, 700000000, 700000000, 100, 100};
        const int add[]  = {0, 1, 0, 0, 2, 0, 0, 0, 0, -1, 1, 0, 0, 0, 0, 0, 0, INT_MAX - 10000, INT_MIN + 10000};
        int len = (int)(sizeof(mul1) / sizeof(mul1[0]));
        call_fma(case_num++, out_buf, mul1, mul2, add, len);
    }

    /* fma_array with len == 0: the for-loop body never runs. This is a
       harmless empty computation; only the unit's own VLA in driver()
       requires a strictly positive length, not this plain int parameter. */
    {
        const int mul1[] = {1};
        const int mul2[] = {1};
        const int add[]  = {1};
        call_fma(case_num++, out_buf, mul1, mul2, add, 0);
    }

    /* fma_array with a negative len: same empty-loop path. */
    {
        const int mul1[] = {1};
        const int mul2[] = {1};
        const int add[]  = {1};
        call_fma(case_num++, out_buf, mul1, mul2, add, -5);
    }

    /* fma_array with out aliasing mul1/mul2/add, mirroring how driver()
       calls it through inner(): out[i] = out[i]*out[i] + out[i]. Values
       are kept small enough that squaring plus itself cannot overflow. */
    {
        int alias_buf[] = {0, 1, -1, 2, -2, 3, -3, 10, -10, 100, -100, 20000, -20000};
        int len = (int)(sizeof(alias_buf) / sizeof(alias_buf[0]));
        call_fma(case_num++, alias_buf, alias_buf, alias_buf, alias_buf, len);
    }

    /* fma_array with len == 1, the smallest non-empty size. */
    {
        const int mul1[] = {7};
        const int mul2[] = {6};
        const int add[]  = {1};
        call_fma(case_num++, out_buf, mul1, mul2, add, 1);
    }

    /* driver(): the unit declares a variable-length array `int
       out[len]`, and a VLA whose size is not strictly positive is
       undefined behavior in C (UndefinedBehaviorSanitizer's vla-bound
       check flags it), so every call below uses len >= 1. driver()
       squares each element against itself (via inner()'s aliased call
       to fma_array), so data values are kept small enough that
       data[i]*data[i]+data[i] cannot overflow int. */
    {
        const int data[] = {0};
        call_driver(case_num++, data, 1);
    }
    {
        const int data[] = {1};
        call_driver(case_num++, data, 1);
    }
    {
        const int data[] = {-1};
        call_driver(case_num++, data, 1);
    }
    {
        const int data[] = {0, 1, -1};
        call_driver(case_num++, data, 3);
    }
    {
        const int data[] = {2, -2, 3, -3, 10, -10};
        call_driver(case_num++, data, 6);
    }
    {
        const int data[] = {20000, -20000, 0, 1, -1, 100, -100, 5, -5, 7};
        call_driver(case_num++, data, 10);
    }
    {
        int data[64];
        for (int i = 0; i < 64; i++) {
            data[i] = (i % 2 == 0) ? (i - 32) : -(i - 32);
        }
        call_driver(case_num++, data, 64);
    }

    return 0;
}
