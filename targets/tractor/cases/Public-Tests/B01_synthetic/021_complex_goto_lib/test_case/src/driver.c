// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <stdlib.h>

void driver(int x, int y) {
    while (x > 0 || y > 0) {
        printf("loop\n");

        if (x == 1 && y == 4) {
            goto label2;
        }

label1:
        if (x > 0) {
            printf("x\n");
            x--;
        }
        
label2:
        if (y == 0) {
            continue;
        }
        printf("y\n");
        y--;
        if (x < 3) {
            goto label1;
        }
    }
}
