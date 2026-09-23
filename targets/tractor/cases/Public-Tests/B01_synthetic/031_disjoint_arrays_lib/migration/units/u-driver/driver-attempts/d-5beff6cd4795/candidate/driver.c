#include "driver.h"

#include <stdio.h>
#include <string.h>
#include <limits.h>

extern int call_fma(const int *data, int len);
extern void fma_array(int *restrict out, const int *mul1, const int *mul2, const int *add, int len);

static void call_fma_array(int case_num, int *out, const int *mul1, const int *mul2, const int *add, int len) {
    printf("case %d fma_array(len=%d)\n", case_num, len);
    fma_array(out, mul1, mul2, add, len);
    for (int i = 0; i < len; i++) {
        printf("case %d out[%d]=%d\n", case_num, i, out[i]);
    }
}

static void call_call_fma(int case_num, const int *data, int len) {
    printf("case %d call_fma(len=%d) ret=%d\n", case_num, len, call_fma(data, len));
}

static void call_driver(int case_num, const char *in) {
    printf("case %d driver(in=\"%s\")\n", case_num, in);
    driver(in);
}

/* Appends the decimal digits of a non-negative value (always small here,
   well under 1000) followed by a single space to buf, using only
   string-handling functions and plain arithmetic, to build long numeric
   input strings for driver() without any disallowed formatting function. */
static void append_uint_and_space(char *buf, unsigned int value) {
    char rev[16];
    char fwd[16];
    int rpos = 0;
    int fpos = 0;

    if (value == 0) {
        rev[rpos++] = '0';
    } else {
        while (value > 0) {
            rev[rpos++] = (char)('0' + (value % 10));
            value /= 10;
        }
    }
    while (rpos > 0) {
        fwd[fpos++] = rev[--rpos];
    }
    fwd[fpos] = '\0';

    strcat(buf, fwd);
    strcat(buf, " ");
}

int main(void) {
    int case_num = 1;

    /* Direct fma_array calls. out is always a buffer distinct from
       mul1/mul2/add: fma_array's out parameter is restrict-qualified,
       so aliasing it with any of the other pointers would itself be
       undefined behavior. Values are chosen so mul1[i]*mul2[i]+add[i]
       never overflows int, including rows landing exactly on
       INT_MAX/INT_MIN and rows with large-magnitude operands. */
    {
        const int mul1[] = {0, 1, -1, -1, 5, INT_MAX, INT_MIN, INT_MAX, INT_MIN, INT_MAX, INT_MIN, 46340, -46340, 2, -2, 3, -3, 100, -100};
        const int mul2[] = {0, 1, 1, -1, -3, 1, 1, 0, 0, 1, 1, 46340, 46340, 1000000000, 1000000000, 700000000, 700000000, 100, 100};
        const int add[]  = {0, 1, 0, 0, 2, 0, 0, 0, 0, -1, 1, 0, 0, 0, 0, 0, 0, INT_MAX - 10000, INT_MIN + 10000};
        int out[19];
        int len = (int)(sizeof(mul1) / sizeof(mul1[0]));
        call_fma_array(case_num++, out, mul1, mul2, add, len);
    }

    /* fma_array with len == 0 and a negative len: the for-loop body
       never runs, a harmless empty computation (fma_array has no VLA,
       only a plain bounded for loop). */
    {
        const int mul1[] = {1};
        const int mul2[] = {1};
        const int add[]  = {1};
        int out[1];
        call_fma_array(case_num++, out, mul1, mul2, add, 0);
    }
    {
        const int mul1[] = {1};
        const int mul2[] = {1};
        const int add[]  = {1};
        int out[1];
        call_fma_array(case_num++, out, mul1, mul2, add, -5);
    }

    /* fma_array with mul1, mul2 and add all aliasing the same source
       buffer (legal: only out carries the restrict qualifier), and out
       a separate buffer, so out[i] = src[i]*src[i]+src[i]. Values kept
       small enough that squaring plus itself cannot overflow. */
    {
        const int src[] = {0, 1, -1, 2, -2, 3, -3, 10, -10, 100, -100, 20000, -20000};
        int out[13];
        int len = (int)(sizeof(src) / sizeof(src[0]));
        call_fma_array(case_num++, out, src, src, src, len);
    }

    /* fma_array with len == 1, the smallest non-empty size. */
    {
        const int mul1[] = {7};
        const int mul2[] = {6};
        const int add[]  = {1};
        int out[1];
        call_fma_array(case_num++, out, mul1, mul2, add, 1);
    }

    /* Direct call_fma calls. Internally it computes out[i] =
       1*data[i]+0, so no multiplication/addition here can ever
       overflow regardless of data's magnitude; only len must stay
       non-negative, since a negative len (other than the len==0 fast
       path) would reach the unit's `int out[len]` VLA with a
       non-positive size, which is undefined behavior. */
    call_call_fma(case_num++, (const int[]){0}, 0);
    call_call_fma(case_num++, (const int[]){0}, 1);
    call_call_fma(case_num++, (const int[]){1}, 1);
    call_call_fma(case_num++, (const int[]){-1}, 1);
    call_call_fma(case_num++, (const int[]){INT_MAX}, 1);
    call_call_fma(case_num++, (const int[]){INT_MIN}, 1);
    call_call_fma(case_num++, (const int[]){10, 20, 30, 40, 50}, 5);
    call_call_fma(case_num++, (const int[]){INT_MIN, 0, INT_MAX}, 3);
    call_call_fma(case_num++, (const int[]){INT_MAX, 0, INT_MIN}, 3);
    {
        int data[100];
        for (int i = 0; i < 100; i++) {
            data[i] = i - 50;
        }
        call_call_fma(case_num++, data, 100);
    }

    /* driver(): parses up to 100 whitespace-separated decimal integers
       from `in` with sscanf("%d%zn", ...), stopping at the first token
       that fails to parse as %d, then calls call_fma on whatever was
       parsed. Every numeric token below stays within [INT_MIN,
       INT_MAX] so the %d conversion itself cannot overflow (which the
       C standard leaves undefined for scanf). */
    call_driver(case_num++, "");
    call_driver(case_num++, "   ");
    call_driver(case_num++, "hello");
    call_driver(case_num++, "42");
    call_driver(case_num++, "0");
    call_driver(case_num++, "-42");
    call_driver(case_num++, "+7");
    call_driver(case_num++, "007");
    call_driver(case_num++, "2147483647");
    call_driver(case_num++, "-2147483648");
    call_driver(case_num++, "1 2 3 4 5");
    call_driver(case_num++, "1\t2\n3");
    call_driver(case_num++, "   42   ");
    call_driver(case_num++, "42abc");
    call_driver(case_num++, "1 2 abc 4");
    call_driver(case_num++, "-abc");
    call_driver(case_num++, "-1 -2 -3");
    call_driver(case_num++, "2147483647 -2147483648 0");

    {
        char numbers_100[700];
        numbers_100[0] = '\0';
        for (unsigned int i = 0; i < 100; i++) {
            append_uint_and_space(numbers_100, i);
        }
        call_driver(case_num++, numbers_100);
    }
    {
        char numbers_105[700];
        numbers_105[0] = '\0';
        for (unsigned int i = 0; i < 105; i++) {
            append_uint_and_space(numbers_105, i);
        }
        call_driver(case_num++, numbers_105);
    }

    return 0;
}
