// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>

void driver(int x) {
    register int y = 2*x;
    y += 300;
    printf("%d\n", y);
}