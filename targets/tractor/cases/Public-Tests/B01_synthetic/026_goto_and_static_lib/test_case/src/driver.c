// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <stdlib.h>

static int y = 123;

static int multi_stage(int x, int z) {
    int result = 0;
    if (x != 1) {
        printf("Error: x != 1\n");
        result = 1;
        goto fail;
    }

    if (y != 2) {
        printf("Error: x == 1 but y != 2\n");
        result = 2;
        goto fail;
    }

    if (z != 3) {
        printf("Error: x == 1 and y == 2, but z != 3\n");
        result = 3;
        goto fail;
    }

    printf("Ok!\n");
    return result;

fail:
    printf("Operation failed\n");
    return result;
}

void driver(int x, int local_y, int z) {
    y = local_y;
    int result = multi_stage(x, z);
    printf("Result: %d\n", result);
}