// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

#include "loop.h"

/*
    Count up to the passed integer.
*/
void loop(int max_val) {
    for (int i = 0; i <= max_val; i++) {
        printf("%d\n", i);
    }
}
