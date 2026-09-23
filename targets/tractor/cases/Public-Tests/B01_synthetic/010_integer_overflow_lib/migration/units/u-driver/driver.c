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

void printHexCharLine(char charHex);

static int g_case = 0;

static void call_printHexCharLine(char c) {
    g_case++;
    printf("case %d printHexCharLine begin c=%d\n", g_case, (int)c);
    printHexCharLine(c);
    printf("case %d printHexCharLine end\n", g_case);
}

static void call_driver(char data) {
    g_case++;
    printf("case %d driver begin data=%d\n", g_case, (int)data);
    driver(data);
    printf("case %d driver end\n", g_case);
}

int main(void) {
    int i;

    for (i = 0; i <= UCHAR_MAX; i++) {
        call_printHexCharLine((char)i);
    }

    for (i = 0; i <= UCHAR_MAX; i++) {
        call_driver((char)i);
    }

    printf("final_case_count=%d\n", g_case);

    return 0;
}
