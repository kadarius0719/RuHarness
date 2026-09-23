// © 2026 Massachusetts Institute of Technology
// MIT License

#include "long.h"

#include <stdio.h>
#include <stdlib.h>

#define ARRAY_SIZE (256 * 1024) // 1MB assuming sizeof(int) = 4
#define ITERATIONS 2000

// Global array
int array[ARRAY_SIZE];

// Perform expensive arithmetic on each element
void perform_expensive_operations() {
    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        int x = array[i];
        for (int j = 0; j < 100; j++) {
            x = x * 3 + 7;
            x = x ^ (x >> 3);
            x = x - (x << 1);
            x = x / 2 + x % 7;
        }
        array[i] = x;
    }
}

void long_exec(unsigned int seed) {
    srand(seed);

    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        array[i] = rand();
    }

    for (int i = 0; i < ITERATIONS; i++) {
        perform_expensive_operations();
    }

    int xor_result = 0;
    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        xor_result ^= array[i];
    }

    printf("%d\n", xor_result);
    return;
}

