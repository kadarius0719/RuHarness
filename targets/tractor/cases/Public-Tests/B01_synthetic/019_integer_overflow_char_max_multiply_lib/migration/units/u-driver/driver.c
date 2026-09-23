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
void printHexCharLine(char charHex);
void printLine(const char *line);

int main(void)
{
    printf("case 1: direct printLine\n");
    printLine("hello");
    printLine("");
    printLine(NULL);

    printf("case 2: direct printHexCharLine\n");
    printHexCharLine((char)0);
    printHexCharLine((char)1);
    printHexCharLine((char)-1);
    printHexCharLine((char)CHAR_MIN);
    printHexCharLine((char)CHAR_MAX);

    printf("case 3: good()\n");
    good();

    printf("case 4: bad()\n");
    bad();

    printf("case 5: driver(useGood=1)\n");
    driver(1);

    printf("case 6: driver(useGood=0)\n");
    driver(0);

    printf("case 7: driver(useGood=1) again\n");
    driver(1);

    printf("case 8: driver(useGood=-7)\n");
    driver(-7);

    printf("case 9: good() again\n");
    good();

    printf("case 10: bad() again\n");
    bad();

    return 0;
}
