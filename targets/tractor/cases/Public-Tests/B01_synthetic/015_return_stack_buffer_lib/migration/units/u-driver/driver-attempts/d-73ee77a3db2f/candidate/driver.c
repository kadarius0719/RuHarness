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

void bad();
void good();
void printLine(const char *line);

int main(void)
{
    printf("case 1: printLine direct\n");
    printLine("hello");
    printLine("");
    printLine(NULL);

    printf("case 2: good() call 1\n");
    good();

    printf("case 3: bad() call 1\n");
    bad();

    printf("case 4: driver(useGood=1)\n");
    driver(1);

    printf("case 5: driver(useGood=0)\n");
    driver(0);

    printf("case 6: good() call 2\n");
    good();

    printf("case 7: bad() call 2\n");
    bad();

    printf("case 8: driver(useGood=1) again\n");
    driver(1);

    printf("case 9: driver(useGood=-9)\n");
    driver(-9);

    printf("case 10: driver(useGood=0) again\n");
    driver(0);

    return 0;
}
