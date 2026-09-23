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

#include "driver.h"

void bad(void);
void good(void);
void printIntLine(int intNumber);
void printLine(const char *line);

static int g_case = 0;

static void call_printLine(const char *line) {
    g_case++;
    printf("case %d printLine begin\n", g_case);
    printLine(line);
    printf("case %d printLine end\n", g_case);
}

static void call_printIntLine(int n) {
    g_case++;
    printf("case %d printIntLine begin n=%d\n", g_case, n);
    printIntLine(n);
    printf("case %d printIntLine end\n", g_case);
}

static void call_good(void) {
    g_case++;
    printf("case %d good begin\n", g_case);
    good();
    printf("case %d good end\n", g_case);
}

static void call_bad(void) {
    g_case++;
    printf("case %d bad begin\n", g_case);
    bad();
    printf("case %d bad end\n", g_case);
}

static void call_driver(void) {
    g_case++;
    printf("case %d driver begin\n", g_case);
    driver();
    printf("case %d driver end\n", g_case);
}

int main(void) {
    int i;

    call_printLine("Hello, RuHarness!");
    call_printLine("");
    call_printLine("Line with numbers 1234567890 and symbols !@#$%^&*()");
    call_printLine(NULL);
    call_printLine("A");
    call_printLine("The quick brown fox jumps over the lazy dog 0123456789 !@#$%^&*()_+-=[]{}|;:,.<>?/~`");
    call_printLine("\n");

    call_printIntLine(0);
    call_printIntLine(1);
    call_printIntLine(-1);
    call_printIntLine(INT_MAX);
    call_printIntLine(INT_MIN);
    call_printIntLine(42);
    call_printIntLine(-42);

    for (i = 0; i < 5; i++) {
        call_good();
    }

    for (i = 0; i < 5; i++) {
        call_bad();
    }

    for (i = 0; i < 5; i++) {
        call_driver();
    }

    printf("final_case_count=%d\n", g_case);

    return 0;
}
