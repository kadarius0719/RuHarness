#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <limits.h>

#include "switch-arith.h"

/* perform_operations has external linkage in the unit but is not
   declared in switch-arith.h (only switch_arith is); the ABI contract
   gives its exact C signature, so we declare it here ourselves. This is
   a plain declaration, never a definition, macro, or address-of use of
   the unit's symbol. */
unsigned int perform_operations(unsigned int a, unsigned int b);

static uint32_t g_rng_state = 0xDEADBEEFu;

static uint32_t xorshift32(void) {
    uint32_t x = g_rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    g_rng_state = x;
    return x;
}

int main(void) {
    int case_no = 0;
    int i;

    /* ---- perform_operations: fixed edge-case vectors, exhaustively
       paired. Every operation inside the unit (add/sub/mul on unsigned
       int, division by a never-zero safe_b, and shifts by b % bitwidth)
       is well defined for any pair of unsigned ints, so no combination
       here can trigger undefined behavior. ---- */
    {
        unsigned int vals[] = {
            0u, 1u, 2u, 31u, 32u, 33u, 0x80000000u, UINT_MAX, UINT_MAX - 1u
        };
        size_t nv = sizeof(vals) / sizeof(vals[0]);
        size_t j, k;
        for (j = 0; j < nv; j++) {
            for (k = 0; k < nv; k++) {
                unsigned int a = vals[j];
                unsigned int b = vals[k];
                unsigned int r;
                printf("case %d perform_operations a=%u b=%u\n", case_no, a, b);
                r = perform_operations(a, b);
                printf("case %d ret=%u\n", case_no, r);
                case_no++;
            }
        }
    }

    /* ---- perform_operations: pseudo-random full-range vectors ---- */
    for (i = 0; i < 40; i++) {
        unsigned int a = xorshift32();
        unsigned int b = xorshift32();
        unsigned int r;
        printf("case %d perform_operations a=%u b=%u\n", case_no, a, b);
        r = perform_operations(a, b);
        printf("case %d ret=%u\n", case_no, r);
        case_no++;
    }

    /* ---- switch_arith: fixed edge-case seeds ---- */
    {
        unsigned int seeds[] = {
            0u, 1u, 2u, 9u, 10u, 100u, UINT_MAX, UINT_MAX - 1u,
            0x80000000u, 0x7FFFFFFFu
        };
        size_t ns = sizeof(seeds) / sizeof(seeds[0]);
        size_t k;
        for (k = 0; k < ns; k++) {
            printf("case %d switch_arith seed=%u\n", case_no, seeds[k]);
            switch_arith(seeds[k]);
            printf("case %d switch_arith_end\n", case_no);
            case_no++;
        }
    }

    /* ---- switch_arith: pseudo-random seeds, to spread coverage across
       every branch of the message-selecting switch (result % 10) ---- */
    for (i = 0; i < 60; i++) {
        unsigned int seed = xorshift32();
        printf("case %d switch_arith seed=%u\n", case_no, seed);
        switch_arith(seed);
        printf("case %d switch_arith_end\n", case_no);
        case_no++;
    }

    return 0;
}
