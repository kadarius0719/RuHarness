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

typedef struct {
    int floors;
    int bedrooms;
    double bathrooms;
} house_t;

void run(house_t *the_house, int extra_bedrooms);

int main(void)
{
    printf("case 1: direct run, extra_bedrooms=0\n");
    {
        house_t h;
        h.floors = 1;
        h.bedrooms = 0;
        h.bathrooms = 0.0;
        run(&h, 0);
    }

    printf("case 2: direct run, extra_bedrooms=3\n");
    {
        house_t h;
        h.floors = 2;
        h.bedrooms = 5;
        h.bathrooms = 2.5;
        run(&h, 3);
    }

    printf("case 3: direct run, extra_bedrooms=-4\n");
    {
        house_t h;
        h.floors = 5;
        h.bedrooms = 10;
        h.bathrooms = 1.5;
        run(&h, -4);
    }

    printf("case 4: direct run, extra_bedrooms=INT_MAX, bedrooms=0\n");
    {
        house_t h;
        h.floors = 0;
        h.bedrooms = 0;
        h.bathrooms = -3.75;
        run(&h, INT_MAX);
    }

    printf("case 5: direct run, extra_bedrooms=INT_MIN, bedrooms=0\n");
    {
        house_t h;
        h.floors = 100;
        h.bedrooms = 0;
        h.bathrooms = 9999.25;
        run(&h, INT_MIN);
    }

    printf("case 6: driver(\"0\")\n");
    driver("0");

    printf("case 7: driver(\"5\")\n");
    driver("5");

    printf("case 8: driver(\"-5\")\n");
    driver("-5");

    printf("case 9: driver(\"100\")\n");
    driver("100");

    printf("case 10: driver(\"-100\")\n");
    driver("-100");

    printf("case 11: driver(\"500000000\")\n");
    driver("500000000");

    printf("case 12: driver(\"-500000000\")\n");
    driver("-500000000");

    printf("case 13: driver(\"abc\")\n");
    driver("abc");

    printf("case 14: driver(\"\")\n");
    driver("");

    printf("case 15: driver(\"99999999999999\")\n");
    driver("99999999999999");

    printf("case 16: driver(\" 42\")\n");
    driver(" 42");

    printf("case 17: driver(\"3.14\")\n");
    driver("3.14");

    printf("case 18: driver(\"0\") again\n");
    driver("0");

    return 0;
}
