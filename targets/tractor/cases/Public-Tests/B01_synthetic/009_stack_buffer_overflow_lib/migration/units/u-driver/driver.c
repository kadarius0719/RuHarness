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

void bad(int data);
void good(int data);
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

static void call_good(int data) {
    g_case++;
    printf("case %d good begin data=%d\n", g_case, data);
    good(data);
    printf("case %d good end\n", g_case);
}

static void call_bad(int data) {
    g_case++;
    printf("case %d bad begin data=%d\n", g_case, data);
    bad(data);
    printf("case %d bad end\n", g_case);
}

static void call_driver(int goodData, int badData) {
    g_case++;
    printf("case %d driver begin goodData=%d badData=%d\n", g_case, goodData, badData);
    driver(goodData, badData);
    printf("case %d driver end\n", g_case);
}

int main(void) {
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

    call_good(0);
    call_good(1);
    call_good(9);
    call_good(-1);
    call_good(-100);
    call_good(10);
    call_good(100);
    call_good(INT_MAX);
    call_good(INT_MIN);

    call_bad(0);
    call_bad(1);
    call_bad(9);
    call_bad(-1);
    call_bad(-100);
    call_bad(INT_MIN);

    call_driver(0, 0);
    call_driver(9, 9);
    call_driver(-1, -1);
    call_driver(INT_MAX, 0);
    call_driver(INT_MIN, -1);
    call_driver(100, 5);
    call_driver(-100, 9);

    printf("final_case_count=%d\n", g_case);

    return 0;
}
