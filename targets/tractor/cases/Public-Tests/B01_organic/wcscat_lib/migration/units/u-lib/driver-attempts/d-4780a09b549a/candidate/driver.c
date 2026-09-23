#include "lib.h"

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

#define CAP 48

static uint32_t xs32(uint32_t *state) {
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

static void fill_str(wchar_t *buf, size_t cap, size_t len, uint32_t *state) {
    size_t i;
    for (i = 0; i < len && i < cap; i++) {
        buf[i] = (wchar_t)('A' + (int)(xs32(state) % 26u));
    }
    if (len < cap) {
        buf[len] = 0;
    }
}

static void print_wstr(int *case_no, const wchar_t *s, size_t cap, int ret,
                        size_t numElem) {
    size_t i;
    printf("case %d numElem=%zu ret=%d dst=[", *case_no, numElem, ret);
    for (i = 0; i < cap; i++) {
        if (i) {
            printf(",");
        }
        printf("%ld", (long)s[i]);
        if (s[i] == 0) {
            break;
        }
    }
    printf("]\n");
    (*case_no)++;
}

static void run_case(int *case_no, wchar_t *dst, size_t numElem,
                      const wchar_t *src) {
    int ret = wcscat(dst, numElem, src);
    print_wstr(case_no, dst, CAP, ret, numElem);
}

int main(void) {
    int case_no = 0;
    uint32_t state = 2463534242u;

    /* A1: null dst */
    {
        wchar_t src[CAP];
        fill_str(src, CAP, 3, &state);
        int ret = wcscat(NULL, 5, src);
        printf("case %d numElem=5 ret=%d dst=[null]\n", case_no, ret);
        case_no++;
    }

    /* A2: numElem == 0, dst left untouched */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        fill_str(src, CAP, 3, &state);
        run_case(&case_no, dst, 0, src);
    }

    /* A3: null src on an empty dst */
    {
        wchar_t dst[CAP];
        memset(dst, 0, sizeof(dst));
        run_case(&case_no, dst, 10, NULL);
    }

    /* A4: null src on a non-empty dst */
    {
        wchar_t dst[CAP];
        memset(dst, 0, sizeof(dst));
        dst[0] = L'A';
        dst[1] = L'B';
        dst[2] = 0;
        run_case(&case_no, dst, 10, NULL);
    }

    /* B1: append onto an empty dst */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        fill_str(src, CAP, 2, &state);
        run_case(&case_no, dst, 20, src);
    }

    /* B2: append onto a non-empty dst with plenty of room */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        dst[0] = L'A';
        dst[1] = L'B';
        dst[2] = 0;
        fill_str(src, CAP, 2, &state);
        run_case(&case_no, dst, 20, src);
    }

    /* B3: exact fit, empty dst, capacity == srclen + 1 */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        fill_str(src, CAP, 5, &state);
        run_case(&case_no, dst, 6, src);
    }

    /* B4: exact fit, non-empty dst */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        dst[0] = L'X';
        dst[1] = L'Y';
        dst[2] = L'Z';
        dst[3] = 0;
        fill_str(src, CAP, 4, &state);
        run_case(&case_no, dst, 8, src);
    }

    /* C1: exactly one short, empty dst */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        fill_str(src, CAP, 5, &state);
        run_case(&case_no, dst, 5, src);
    }

    /* C2: existing content fills entire capacity, no terminator */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        size_t i;
        for (i = 0; i < CAP; i++) {
            dst[i] = L'X';
        }
        fill_str(src, CAP, 3, &state);
        run_case(&case_no, dst, 10, src);
    }

    /* C3: numElem == 1, existing nonzero char, no terminator */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        size_t i;
        for (i = 0; i < CAP; i++) {
            dst[i] = L'Q';
        }
        fill_str(src, CAP, 3, &state);
        run_case(&case_no, dst, 1, src);
    }

    /* C4: numElem == 1, empty dst, empty src -> fits exactly */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        fill_str(src, CAP, 0, &state);
        run_case(&case_no, dst, 1, src);
    }

    /* C5: numElem == 1, empty dst, one-char src -> overflow */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        fill_str(src, CAP, 1, &state);
        run_case(&case_no, dst, 1, src);
    }

    /* D: empty src onto non-empty dst, exact-fit and oversized */
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        dst[0] = L'M';
        dst[1] = L'N';
        dst[2] = 0;
        fill_str(src, CAP, 0, &state);
        run_case(&case_no, dst, 3, src);
    }
    {
        wchar_t dst[CAP];
        wchar_t src[CAP];
        memset(dst, 0, sizeof(dst));
        dst[0] = L'M';
        dst[1] = L'N';
        dst[2] = 0;
        fill_str(src, CAP, 0, &state);
        run_case(&case_no, dst, CAP, src);
    }

    /* E: deterministic random sweep across lengths and capacities */
    {
        int trial;
        for (trial = 0; trial < 250; trial++) {
            wchar_t dst[CAP];
            wchar_t src[CAP];
            size_t dst_len = xs32(&state) % 9u;
            size_t src_len = xs32(&state) % 9u;
            size_t need = dst_len + src_len + 1;
            size_t choice = xs32(&state) % 3u;
            size_t numElem;
            memset(dst, 0, sizeof(dst));
            fill_str(dst, CAP, dst_len, &state);
            fill_str(src, CAP, src_len, &state);
            if (choice == 0 && need > 1) {
                numElem = need - 1; /* too small by one */
            } else if (choice == 1) {
                numElem = need; /* exact fit */
            } else {
                numElem = need + (xs32(&state) % 10u); /* oversized */
            }
            if (numElem > CAP) {
                numElem = CAP;
            }
            run_case(&case_no, dst, numElem, src);
        }
    }

    return 0;
}
