#include "driver.h"

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

void bad(float data);
void good(float data);
void printIntLine(int intNumber);
void printLine(const char *line);

int main(void)
{
    printf("case 1: direct printLine\n");
    printLine("hello from driver");
    printLine("");
    printLine(NULL);

    printf("case 2: direct printIntLine\n");
    printIntLine(0);
    printIntLine(1);
    printIntLine(-1);
    printIntLine(2147483647);
    printIntLine(-2147483647 - 1);

    printf("case 3: good(0.0f)\n");
    good(0.0f);

    printf("case 4: good(0.0000005f)\n");
    good(0.0000005f);

    printf("case 5: good(0.001f)\n");
    good(0.001f);

    printf("case 6: good(5.0f)\n");
    good(5.0f);

    printf("case 7: good(-3.0f)\n");
    good(-3.0f);

    printf("case 8: bad(4.0f)\n");
    bad(4.0f);

    printf("case 9: bad(-2.0f)\n");
    bad(-2.0f);

    printf("case 10: bad(0.5f)\n");
    bad(0.5f);

    printf("case 11: bad(1000.0f)\n");
    bad(1000.0f);

    printf("case 12: driver(0.0f, 4.0f)\n");
    driver(0.0f, 4.0f);

    printf("case 13: driver(5.0f, -2.0f)\n");
    driver(5.0f, -2.0f);

    printf("case 14: driver(0.0000005f, 0.5f)\n");
    driver(0.0000005f, 0.5f);

    printf("case 15: driver(-3.0f, 1000.0f)\n");
    driver(-3.0f, 1000.0f);

    return 0;
}
