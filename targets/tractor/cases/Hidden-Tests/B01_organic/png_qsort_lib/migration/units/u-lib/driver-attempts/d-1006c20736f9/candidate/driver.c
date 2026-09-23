#include "lib.h"
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <inttypes.h>
#include <limits.h>
#include <float.h>
#include <math.h>
#include <stdbool.h>
#include <ctype.h>
#include <errno.h>

#define MAXN 24

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static int case_id = 0;

static void print_items(const char *label, const cp_integer_image_t *items, int count) {
    int i;
    printf("case %d %s count=%d", case_id++, label, count);
    for (i = 0; i < count; i++) {
        printf(" [idx=%d sx=%d sy=%d minx=%d miny=%d maxx=%d maxy=%d fit=%d]",
               items[i].img_index, items[i].size.x, items[i].size.y,
               items[i].min.x, items[i].min.y, items[i].max.x, items[i].max.y,
               items[i].fit);
    }
    printf("\n");
}

static void run_case(const char *label, const int *sx, const int *sy, int count) {
    cp_integer_image_t items[MAXN];
    int i;
    for (i = 0; i < count; i++) {
        items[i].img_index = i;
        items[i].size.x = sx[i];
        items[i].size.y = sy[i];
        items[i].min.x = -sx[i];
        items[i].min.y = -sy[i];
        items[i].max.x = sx[i] + 1;
        items[i].max.y = sy[i] + 1;
        items[i].fit = i * 2 + 1;
    }
    qsort(items, count);
    print_items(label, items, count);
}

int main(void) {
    uint32_t rng = 1122334455u;
    int i, r;

    {
        int sx[1] = {5}, sy[1] = {5};
        run_case("count0", sx, sy, 0);
    }
    {
        int sx[1] = {7}, sy[1] = {3};
        run_case("count1", sx, sy, 1);
        run_case("count_negative", sx, sy, -5);
    }
    {
        int sx[2] = {1, 9}, sy[2] = {1, 9};
        run_case("count2_asc", sx, sy, 2);
    }
    {
        int sx[2] = {9, 1}, sy[2] = {9, 1};
        run_case("count2_desc", sx, sy, 2);
    }
    {
        int sx[3] = {2, 8, 5}, sy[3] = {2, 8, 5};
        run_case("count3_mixed", sx, sy, 3);
    }
    {
        int sx[4] = {4, 4, 4, 4}, sy[4] = {4, 4, 4, 4};
        run_case("count4_equal_perimeter", sx, sy, 4);
    }
    {
        int sx[6] = {1, 2, 3, 4, 5, 6}, sy[6] = {1, 2, 3, 4, 5, 6};
        run_case("count6_ascending", sx, sy, 6);
    }
    {
        int sx[6] = {6, 5, 4, 3, 2, 1}, sy[6] = {6, 5, 4, 3, 2, 1};
        run_case("count6_descending", sx, sy, 6);
    }
    {
        int sx[8] = {0, -1, 1, -2, 2, -3, 3, 0}, sy[8] = {0, 1, -1, 2, -2, 3, -3, 0};
        run_case("count8_negative_mix", sx, sy, 8);
    }
    {
        int sx[10] = {5, 3, 8, 1, 9, 2, 7, 4, 6, 0};
        int sy[10] = {5, 3, 8, 1, 9, 2, 7, 4, 6, 0};
        run_case("count10_shuffled", sx, sy, 10);
    }
    {
        int sx[5] = {1000, -1000, 500, -500, 0};
        int sy[5] = {1000, -1000, 500, -500, 0};
        run_case("count5_extremes", sx, sy, 5);
    }

    for (r = 0; r < 12; r++) {
        int sx[MAXN], sy[MAXN];
        int n = 1 + (int)(xorshift32(&rng) % (uint32_t)(MAXN - 1));
        for (i = 0; i < n; i++) {
            sx[i] = (int)((int32_t)(xorshift32(&rng) % 2001u) - 1000);
            sy[i] = (int)((int32_t)(xorshift32(&rng) % 2001u) - 1000);
        }
        run_case("random", sx, sy, n);
    }

    printf("total_cases=%d\n", case_id);
    return 0;
}
