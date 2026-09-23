#include "lib.h"

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

#define MAX_N 100

static uint64_t g_rng_state = 0x2545F4914F6CDD1DULL;

static uint64_t xorshift64(void) {
    uint64_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    g_rng_state = x;
    return x;
}

static int g_case = 0;

static void print_array(const char *label, const spritebatch_sprite_t *arr, int n) {
    int i;
    printf("case %d %s=[", g_case, label);
    for (i = 0; i < n; i++) {
        if (i != 0) {
            printf(" ");
        }
        printf("%" PRIu64 ":%d", (uint64_t)arr[i].texture_id, arr[i].sort_bits);
    }
    printf("]\n");
}

static void run_case(const char *tag, spritebatch_sprite_t *a, int n) {
    static spritebatch_sprite_t scratch[MAX_N];

    memset(scratch, 0, sizeof(scratch));
    merge_sort(a, scratch, n);

    printf("case %d tag=%s n=%d\n", g_case, tag, n);
    print_array("a", a, n);
    print_array("b", scratch, n);
    g_case++;
}

static void fill_ascending(spritebatch_sprite_t *arr, int n) {
    int i;
    for (i = 0; i < n; i++) {
        arr[i].sort_bits = i;
        arr[i].texture_id = (unsigned long long)(i * 7 + 3);
    }
}

static void fill_descending(spritebatch_sprite_t *arr, int n) {
    int i;
    for (i = 0; i < n; i++) {
        arr[i].sort_bits = n - 1 - i;
        arr[i].texture_id = (unsigned long long)(i * 5 + 1);
    }
}

static void fill_random(spritebatch_sprite_t *arr, int n) {
    int i;
    for (i = 0; i < n; i++) {
        arr[i].sort_bits = (int)(int32_t)(xorshift64() & 0xFFFFFFFFULL);
        arr[i].texture_id = (unsigned long long)xorshift64();
    }
}

static void fill_tie_sort_bits(spritebatch_sprite_t *arr, int n) {
    int i;
    for (i = 0; i < n; i++) {
        arr[i].sort_bits = 42;
        arr[i].texture_id = (unsigned long long)(n - i);
    }
}

static void fill_tie_texture_id(spritebatch_sprite_t *arr, int n) {
    int i;
    for (i = 0; i < n; i++) {
        arr[i].sort_bits = (int)((i * 13) % 37) - 18;
        arr[i].texture_id = 1000ULL;
    }
}

int main(void) {
    static const int sizes[] = {
        0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33
    };
    static const int extra_sizes[] = { 64, 100 };
    size_t ns = sizeof(sizes) / sizeof(sizes[0]);
    size_t ne = sizeof(extra_sizes) / sizeof(extra_sizes[0]);
    size_t si;
    static spritebatch_sprite_t buf[MAX_N];

    for (si = 0; si < ns; si++) {
        int n = sizes[si];

        fill_ascending(buf, n);
        run_case("asc", buf, n);

        fill_descending(buf, n);
        run_case("desc", buf, n);

        fill_random(buf, n);
        run_case("rand", buf, n);
    }

    for (si = 0; si < ne; si++) {
        int n = extra_sizes[si];
        fill_random(buf, n);
        run_case("rand_big", buf, n);
    }

    fill_tie_sort_bits(buf, 8);
    run_case("tie_sort_bits", buf, 8);

    fill_tie_texture_id(buf, 8);
    run_case("tie_texture_id", buf, 8);

    return 0;
}
