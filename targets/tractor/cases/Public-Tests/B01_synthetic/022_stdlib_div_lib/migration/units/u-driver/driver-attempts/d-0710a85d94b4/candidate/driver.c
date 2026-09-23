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

int main(void)
{
    printf("case 1: driver(10, 3)\n");
    driver(10, 3);

    printf("case 2: driver(-10, 3)\n");
    driver(-10, 3);

    printf("case 3: driver(10, -3)\n");
    driver(10, -3);

    printf("case 4: driver(-10, -3)\n");
    driver(-10, -3);

    printf("case 5: driver(0, 5)\n");
    driver(0, 5);

    printf("case 6: driver(1, 1)\n");
    driver(1, 1);

    printf("case 7: driver(INT_MAX, 1)\n");
    driver(INT_MAX, 1);

    printf("case 8: driver(INT_MIN, 1)\n");
    driver(INT_MIN, 1);

    printf("case 9: driver(INT_MAX, -1)\n");
    driver(INT_MAX, -1);

    printf("case 10: driver(5, INT_MAX)\n");
    driver(5, INT_MAX);

    printf("case 11: driver(-5, INT_MIN)\n");
    driver(-5, INT_MIN);

    printf("case 12: driver(INT_MIN, 2)\n");
    driver(INT_MIN, 2);

    printf("case 13: driver(INT_MIN, INT_MAX)\n");
    driver(INT_MIN, INT_MAX);

    printf("case 14: driver(7, 7)\n");
    driver(7, 7);

    printf("case 15: driver(-7, 7)\n");
    driver(-7, 7);

    return 0;
}
