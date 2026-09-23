#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <limits.h>

#include "driver.h"

/* fma_array has external linkage in the unit but is not declared in
   driver.h (only driver is); the ABI contract gives its exact C
   signature, so we declare it here ourselves. This is a plain
   declaration, never a definition, macro, or address-of use of the
   unit's symbol. */
void fma_array(int *out, const int *mul1, const int *mul2, const int *add, int len);

static uint32_t g_rng_state = 0x6D2B79F5u;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

static int rand_range(int lo, int hi) {
    uint32_t span = (uint32_t)(hi - lo) + 1u;
    uint32_t r = xorshift32() % span;
    return lo + (int)r;
}

#define DATA_CAP 200

static void run_driver_case(int case_no, const int *data, int len) {
    int i;
    printf("case %d driver len=%d data=[", case_no, len);
    for (i = 0; i < len; i++) {
        printf("%s%d", i == 0 ? "" : ",", data[i]);
    }
    printf("]\n");
    driver(data, len);
    printf("case %d driver_end\n", case_no);
}

static void run_fma_case(int case_no, const int *mul1, const int *mul2, const int *add, int len) {
    int out[DATA_CAP];
    int i;
    printf("case %d fma_array len=%d\n", case_no, len);
    fma_array(out, mul1, mul2, add, len);
    for (i = 0; i < len; i++) {
        printf("case %d out[%d]=%d\n", case_no, i, out[i]);
    }
    printf("case %d fma_array_end\n", case_no);
}

int main(void) {
    int case_no = 0;
    int i;
    int data[DATA_CAP];

    /* ---- driver: fixed, small, overflow-safe vectors. driver() declares
       a VLA sized `len`, so len must be strictly positive (len <= 0 is
       undefined behavior per the C standard's VLA-bound rule, and would
       also be flagged by UndefinedBehaviorSanitizer's vla-bound check);
       and since driver forwards data as mul1, mul2 AND add to
       fma_array, out[i] == data[i]*data[i] + data[i], so |data[i]| is
       kept small enough that the square can never overflow signed int
       (46340^2 is about INT_MAX, so 30000 leaves a wide safety margin). */
    {
        int d1[] = { 0 };
        run_driver_case(case_no, d1, 1); case_no++;
    }
    {
        int d2[] = { 1 };
        run_driver_case(case_no, d2, 1); case_no++;
    }
    {
        int d3[] = { -1 };
        run_driver_case(case_no, d3, 1); case_no++;
    }
    {
        int d4[] = { 30000 };
        run_driver_case(case_no, d4, 1); case_no++;
    }
    {
        int d5[] = { -30000 };
        run_driver_case(case_no, d5, 1); case_no++;
    }
    {
        int d6[] = { 0, 1 };
        run_driver_case(case_no, d6, 2); case_no++;
    }
    {
        int d7[] = { -1, 0, 1 };
        run_driver_case(case_no, d7, 3); case_no++;
    }
    {
        int d8[] = { -30000, -1, 0, 1, 30000 };
        run_driver_case(case_no, d8, 5); case_no++;
    }

    for (i = 0; i < DATA_CAP; i++) {
        data[i] = rand_range(-30000, 30000);
    }
    run_driver_case(case_no, data, 10); case_no++;
    run_driver_case(case_no, data, 50); case_no++;
    run_driver_case(case_no, data, 100); case_no++;
    run_driver_case(case_no, data, DATA_CAP); case_no++;

    /* several more independent random passes at a moderate length */
    for (i = 0; i < 5; i++) {
        int len = 20;
        int d[20];
        int j;
        for (j = 0; j < len; j++) {
            d[j] = rand_range(-30000, 30000);
        }
        run_driver_case(case_no, d, len); case_no++;
    }

    /* ---- fma_array: direct exercise. Unlike driver(), fma_array takes
       plain pointers (no VLA of its own), so len <= 0 is completely
       safe here: its loop body simply never executes and none of the
       pointers are ever dereferenced. ---- */
    {
        int in_unused[1] = { 0 };
        run_fma_case(case_no, in_unused, in_unused, in_unused, 0);
        case_no++;
    }
    {
        int in_unused[1] = { 0 };
        run_fma_case(case_no, in_unused, in_unused, in_unused, -1);
        case_no++;
    }
    {
        int in_unused[1] = { 0 };
        run_fma_case(case_no, in_unused, in_unused, in_unused, INT_MIN);
        case_no++;
    }

    /* mul1 == 0 makes the product exactly 0 regardless of mul2, so add
       can safely span the full int domain, including INT_MIN/INT_MAX */
    {
        int zero1[3] = { 0, 0, 0 };
        int mul2v[3] = { 1, INT_MIN, INT_MAX };
        int addv[3]  = { 0, INT_MAX, INT_MIN };
        run_fma_case(case_no, zero1, mul2v, addv, 3);
        case_no++;
    }

    /* mul2 == 0 is symmetric: product is 0 regardless of mul1 */
    {
        int mul1v[3] = { 1, INT_MIN, INT_MAX };
        int zero2[3] = { 0, 0, 0 };
        int addv[3]  = { 0, INT_MIN, INT_MAX };
        run_fma_case(case_no, mul1v, zero2, addv, 3);
        case_no++;
    }

    /* add == 0, mul1/mul2 bounded so the product itself cannot overflow */
    {
        int mul1v[4] = { 0, 1, -1, 46340 };
        int mul2v[4] = { 0, 1, -1, 46340 };
        int zeroa[4] = { 0, 0, 0, 0 };
        run_fma_case(case_no, mul1v, mul2v, zeroa, 4);
        case_no++;
    }

    /* len == 1, ordinary small values */
    {
        int m1[1] = { 7 };
        int m2[1] = { 6 };
        int a1[1] = { 5 };
        run_fma_case(case_no, m1, m2, a1, 1);
        case_no++;
    }

    /* pseudo-random, independent, bounded mul1/mul2/add arrays: product
       magnitude stays comfortably below INT_MAX */
    for (i = 0; i < 10; i++) {
        int len = 20;
        int m1[20], m2[20], a[20];
        int j;
        for (j = 0; j < len; j++) {
            m1[j] = rand_range(-1000, 1000);
            m2[j] = rand_range(-1000, 1000);
            a[j]  = rand_range(-1000, 1000);
        }
        run_fma_case(case_no, m1, m2, a, len);
        case_no++;
    }

    return 0;
}
