// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <stdlib.h>

void driver(int x, int y) {
    div_t result = div(x, y);
    printf("quotient: %d, remainder: %d\n", result.quot, result.rem);
}