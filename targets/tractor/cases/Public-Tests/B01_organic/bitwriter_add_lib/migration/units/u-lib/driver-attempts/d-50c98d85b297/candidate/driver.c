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

static uint8_t g_buffer[64];
static int g_case = 0;
static uint32_t xr_state;

static uint32_t xr32(void) {
    uint32_t x = xr_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    xr_state = x;
    return x;
}

static uint64_t xr64(void) {
    uint64_t hi = xr32();
    uint64_t lo = xr32();
    return (hi << 32) | lo;
}

static void reset_bw(tflac_bitwriter *bw) {
    bw->val = 0;
    bw->bits = 0;
    bw->pos = 0;
    bw->len = 0;
    bw->tot = 0;
    bw->buffer = g_buffer;
}

static void print_bw(int ret, const tflac_bitwriter *bw) {
    g_case++;
    printf("case %d ret=%d val=%016" PRIx64 " bits=%" PRIu32 " tot=%" PRIu32 "\n",
           g_case, ret, (uint64_t)bw->val, bw->bits, bw->tot);
}

int main(void) {
    xr_state = 0xC001D00Du;

    /* Scenario A: single call from a fresh state, bits = 1..63, val = 0 */
    for (uint32_t bits = 1; bits <= 63; bits++) {
        tflac_bitwriter bw;
        reset_bw(&bw);
        int ret = bitwriter_add(&bw, bits, (tflac_uint)0);
        print_bw(ret, &bw);
    }

    /* Scenario B: single call from a fresh state, bits = 1..63, val = all-ones for that width */
    for (uint32_t bits = 1; bits <= 63; bits++) {
        tflac_bitwriter bw;
        reset_bw(&bw);
        tflac_uint val = (((tflac_uint)1) << bits) - (tflac_uint)1;
        int ret = bitwriter_add(&bw, bits, val);
        print_bw(ret, &bw);
    }

    /* Scenario C: bits = 64 (full width), val = 0 and all-ones; each is the last call on its bw */
    {
        tflac_bitwriter bw;
        reset_bw(&bw);
        int ret = bitwriter_add(&bw, 64, (tflac_uint)0);
        print_bw(ret, &bw);
    }
    {
        tflac_bitwriter bw;
        reset_bw(&bw);
        int ret = bitwriter_add(&bw, 64, (tflac_uint)0xFFFFFFFFFFFFFFFFULL);
        print_bw(ret, &bw);
    }

    /* Scenario D: chained calls staying strictly below 64 cumulative bits, several streams */
    for (int s = 0; s < 12; s++) {
        tflac_bitwriter bw;
        reset_bw(&bw);
        uint32_t used = 0;
        int steps = 1 + (int)(xr32() % 5u);
        for (int k = 0; k < steps; k++) {
            uint32_t room = 63u - used;
            if (room == 0) {
                break;
            }
            uint32_t bits = 1u + (xr32() % room);
            tflac_uint val = xr64();
            int ret = bitwriter_add(&bw, bits, val);
            used += bits;
            print_bw(ret, &bw);
        }
    }

    /* Scenario E: accumulate then push the total to or past 64 on the final call for that bw */
    for (int s = 0; s < 8; s++) {
        tflac_bitwriter bw;
        reset_bw(&bw);
        uint32_t first_bits = 1u + (xr32() % 63u);
        tflac_uint val1 = xr64();
        int ret1 = bitwriter_add(&bw, first_bits, val1);
        print_bw(ret1, &bw);

        uint32_t room = 63u - first_bits;
        uint32_t second_bits = room + 1u + (xr32() % 20u);
        if (second_bits > 64u) {
            second_bits = 64u;
        }
        if (second_bits < 1u) {
            second_bits = 1u;
        }
        tflac_uint val2 = xr64();
        int ret2 = bitwriter_add(&bw, second_bits, val2);
        print_bw(ret2, &bw);
    }

    /* Scenario F: exact boundary, bw->bits = 63 then a final 1-bit call sums to exactly 64 */
    {
        tflac_bitwriter bw;
        reset_bw(&bw);
        int ret1 = bitwriter_add(&bw, 63, (tflac_uint)0x5A5A5A5A5A5A5A5AULL);
        print_bw(ret1, &bw);
        int ret2 = bitwriter_add(&bw, 1, (tflac_uint)1);
        print_bw(ret2, &bw);
    }

    /* Scenario G: exact boundary sum = 63 (loop never triggered), two chained calls */
    {
        tflac_bitwriter bw;
        reset_bw(&bw);
        int ret1 = bitwriter_add(&bw, 30, (tflac_uint)0x123456789AULL);
        print_bw(ret1, &bw);
        int ret2 = bitwriter_add(&bw, 33, (tflac_uint)0x3FFFFFFFFULL);
        print_bw(ret2, &bw);
    }

    /* Scenario H: many chained 1-bit calls (up to 63) from a fresh state, alternating val 0/1 */
    {
        tflac_bitwriter bw;
        reset_bw(&bw);
        for (int k = 0; k < 63; k++) {
            tflac_uint val = (tflac_uint)(k & 1);
            int ret = bitwriter_add(&bw, 1, val);
            print_bw(ret, &bw);
        }
    }

    return 0;
}
