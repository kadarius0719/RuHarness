#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <limits.h>
#include <stdbool.h>

#include "simplestruct.h"

static uint32_t g_rng_state = 0x243F6A88u;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

static void run_case(int case_no, int month, int day, int year) {
    struct Date d;
    bool is_may;
    int y;
    d.month = month;
    d.day = day;
    d.year = year;
    printf("case %d date month=%d day=%d year=%d\n", case_no, month, day, year);
    is_may = isItMay(d);
    printf("case %d isItMay ret=%d\n", case_no, (int)is_may);
    y = whatYearIsIt(d);
    printf("case %d whatYearIsIt ret=%d\n", case_no, y);
}

int main(void) {
    int case_no = 0;
    int i;

    {
        int fixed[][3] = {
            {5, 1, 2024},
            {5, 31, 0},
            {4, 30, 2024},
            {6, 1, 2024},
            {0, 0, 0},
            {1, 1, 1},
            {12, 31, 9999},
            {-1, -1, -1},
            {INT_MIN, INT_MIN, INT_MIN},
            {INT_MAX, INT_MAX, INT_MAX},
            {5, 0, INT_MIN},
            {5, -5, INT_MAX},
            {5, 5, 5},
            {-5, 5, -5},
            {5, INT_MAX, INT_MIN},
            {5, INT_MIN, INT_MAX},
        };
        size_t n = sizeof(fixed) / sizeof(fixed[0]);
        size_t k;
        for (k = 0; k < n; k++) {
            run_case(case_no, fixed[k][0], fixed[k][1], fixed[k][2]);
            case_no++;
        }
    }

    /* pseudo-random (month, day, year) triples across the full int32
       domain: both functions only read struct fields (an equality test
       and a plain return), so no combination of values is unsafe. */
    for (i = 0; i < 30; i++) {
        int month = (int)xorshift32();
        int day = (int)xorshift32();
        int year = (int)xorshift32();
        run_case(case_no, month, day, year);
        case_no++;
    }

    /* dedicated random sweep with month forced to 5, to reliably exercise
       the true branch of isItMay across varied day/year values */
    for (i = 0; i < 10; i++) {
        int day = (int)xorshift32();
        int year = (int)xorshift32();
        run_case(case_no, 5, day, year);
        case_no++;
    }

    return 0;
}
