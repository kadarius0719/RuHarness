// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>

void driver(int x) {
    for (int i = 0, j = 0; i < x; i++, j += 2) {
        printf("%d %d\n", i, j);
    }
}
