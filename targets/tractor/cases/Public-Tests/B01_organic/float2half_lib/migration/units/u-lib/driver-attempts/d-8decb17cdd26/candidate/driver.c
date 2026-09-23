#include "lib.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>

typedef union {
    uint32_t u;
    float f;
} cvt32_t;

static float bits_to_float(uint32_t bits) {
    cvt32_t c;
    c.u = bits;
    return c.f;
}

static uint32_t rng_state = 0x2545f491u;

static uint32_t xorshift32(void) {
    uint32_t x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static int g_case_id = 1;

static void run_case(uint32_t bits) {
    float f = bits_to_float(bits);
    uint16_t h = float2half(f);
    printf("case %d bits=%" PRIx32 " in=%a ret=%" PRIu16 "\n", g_case_id,
           bits, f, h);
    g_case_id++;
}

int main(void) {
    uint32_t e;
    int i;

    /* Named edge cases. */
    run_case(0x00000000u); /* +0 */
    run_case(0x80000000u); /* -0 */
    run_case(0x3f800000u); /* 1.0 */
    run_case(0xbf800000u); /* -1.0 */
    run_case(0x40000000u); /* 2.0 */
    run_case(0x40490fdbu); /* pi */
    run_case(0x7f7fffffu); /* FLT_MAX */
    run_case(0xff7fffffu); /* -FLT_MAX */
    run_case(0x00800000u); /* smallest normal float */
    run_case(0x80800000u); /* -smallest normal float */
    run_case(0x00000001u); /* smallest denormal float */
    run_case(0x80000001u); /* -smallest denormal float */
    run_case(0x7f800000u); /* +inf */
    run_case(0xff800000u); /* -inf */
    run_case(0x7fc00000u); /* qNaN */
    run_case(0xffc00000u); /* -qNaN */
    run_case(0x477fe000u); /* 65504 = max representable half */
    run_case(0x47800000u); /* 65536 -> overflows half to inf */
    run_case(0x38800000u); /* 2^-14, min normal half */
    run_case(0x33800000u); /* 2^-24, min subnormal half */
    run_case(0x387fc000u); /* just below half normal boundary */
    run_case(0x33000000u); /* below half subnormal threshold, rounds to 0 */

    /* Systematic sweep across every biased exponent and both signs, at
       the minimum and maximum mantissa, to exercise every entry of the
       base/shift lookup tables. */
    for (e = 0; e < 256u; ++e) {
        for (i = 0; i < 2; ++i) {
            uint32_t sign = (uint32_t)i << 31;
            run_case(sign | (e << 23) | 0x00000000u);
            run_case(sign | (e << 23) | 0x007fffffu);
        }
    }

    /* Pseudo-random bit patterns covering arbitrary mantissas. */
    for (i = 0; i < 24; ++i) {
        uint32_t bits = xorshift32();
        run_case(bits);
    }

    return 0;
}
