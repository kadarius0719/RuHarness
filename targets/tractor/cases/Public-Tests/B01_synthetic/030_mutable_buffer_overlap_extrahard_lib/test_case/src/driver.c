// © 2026 Massachusetts Institute of Technology
// MIT License

#include "driver.h"

#include <stdio.h>
#include <string.h>

void fma_array(int *out, const int *mul1, const int *mul2, const int *add, int len) {
    for (int i = 0; i < len; i++) {
        out[i] = mul1[i] * mul2[i] + add[i];
    }
}

static void inner(int *out, int len) {
    fma_array(out, out, out, out, len);
    for (int i = 0; i < len; i++) {
        printf("%d\n", out[i]);
    }
}

void driver(const int *data, int len) {
    int out[len];
    memcpy(out, data, len * sizeof(int));
    inner(out, len);
}
