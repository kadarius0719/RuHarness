#include "slicing.h"

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

static uint32_t xs32(uint32_t *state) {
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

static void run_case(int *case_no, const char *label, char *mystr,
                      int *start_ptr, int *stop_ptr) {
    int ret = slice(mystr, start_ptr, stop_ptr);
    printf("case %d [%s] ret=%d\n", *case_no, label, ret);
    (*case_no)++;
}

int main(void) {
    int case_no = 0;

    {
        char buf[] = "Hello World";
        run_case(&case_no, "full", buf, NULL, NULL);
    }
    {
        char buf[] = "Hello World";
        int st = 5;
        run_case(&case_no, "start_only", buf, &st, NULL);
    }
    {
        char buf[] = "Hello World";
        int sp = 5;
        run_case(&case_no, "stop_only", buf, NULL, &sp);
    }
    {
        char buf[] = "Hello World";
        int st = 2, sp = 7;
        run_case(&case_no, "both", buf, &st, &sp);
    }
    {
        char buf[] = "Hello World";
        int st = 12;
        run_case(&case_no, "start_off_end", buf, &st, NULL);
    }
    {
        char buf[] = "Hello World";
        int st = 11;
        run_case(&case_no, "start_eq_len_stop_null", buf, &st, NULL);
    }
    {
        char buf[] = "Hello World";
        int sp = 12;
        run_case(&case_no, "stop_off_end", buf, NULL, &sp);
    }
    {
        char buf[] = "Hello World";
        int sp = 11;
        run_case(&case_no, "stop_eq_len", buf, NULL, &sp);
    }
    {
        char buf[] = "Hello World";
        int st = 5, sp = 5;
        run_case(&case_no, "stop_eq_start", buf, &st, &sp);
    }
    {
        char buf[] = "Hello World";
        int st = 5, sp = 3;
        run_case(&case_no, "stop_lt_start", buf, &st, &sp);
    }
    {
        char buf[] = "Hello World";
        int st = -1;
        run_case(&case_no, "start_negative", buf, &st, NULL);
    }
    {
        char buf[] = "Hello World";
        int sp = -1;
        run_case(&case_no, "stop_negative", buf, NULL, &sp);
    }
    {
        char buf[] = "";
        run_case(&case_no, "empty_default", buf, NULL, NULL);
    }
    {
        char buf[] = "";
        int st = 0;
        run_case(&case_no, "empty_start0", buf, &st, NULL);
    }
    {
        char buf[] = "";
        int st = 1;
        run_case(&case_no, "empty_start1_off_end", buf, &st, NULL);
    }
    {
        char buf[] = "";
        int sp = 0;
        run_case(&case_no, "empty_stop0_eq_start", buf, NULL, &sp);
    }
    {
        char buf[] = "A";
        run_case(&case_no, "single_default", buf, NULL, NULL);
    }
    {
        char buf[] = "A";
        int st = 0, sp = 1;
        run_case(&case_no, "single_full", buf, &st, &sp);
    }
    {
        char buf[] = "A";
        int st = 0, sp = 0;
        run_case(&case_no, "single_empty_range", buf, &st, &sp);
    }
    {
        char buf[] = "Hello World";
        int st = 0, sp = 11;
        run_case(&case_no, "zero_start_full_stop", buf, &st, &sp);
    }

    {
        char big[201];
        int i;
        for (i = 0; i < 200; i++) {
            big[i] = (char)('a' + (i % 26));
        }
        big[200] = 0;
        run_case(&case_no, "big_default", big, NULL, NULL);
        {
            int st = 100, sp = 150;
            run_case(&case_no, "big_mid", big, &st, &sp);
        }
        {
            int st = 199, sp = 200;
            run_case(&case_no, "big_last_char", big, &st, &sp);
        }
        {
            int st = 200;
            run_case(&case_no, "big_start_eq_len", big, &st, NULL);
        }
        {
            int st = 201;
            run_case(&case_no, "big_start_over", big, &st, NULL);
        }
    }

    {
        uint32_t state = 2463534242u;
        int trial;
        for (trial = 0; trial < 200; trial++) {
            char buf[33];
            int len = (int)(xs32(&state) % 32u) + 1;
            int k;
            for (k = 0; k < len; k++) {
                buf[k] = (char)('a' + (int)(xs32(&state) % 26u));
            }
            buf[len] = 0;
            {
                int use_start = (int)(xs32(&state) % 2u);
                int use_stop = (int)(xs32(&state) % 2u);
                int st = 0;
                int sp = 0;
                int *stp = NULL;
                int *spp = NULL;
                if (use_start) {
                    st = (int)(xs32(&state) % (uint32_t)(len + 3)) - 1;
                    stp = &st;
                }
                if (use_stop) {
                    sp = (int)(xs32(&state) % (uint32_t)(len + 3)) - 1;
                    spp = &sp;
                }
                run_case(&case_no, "random", buf, stp, spp);
            }
        }
    }

    return 0;
}
